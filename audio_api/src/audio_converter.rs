use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use service_protos::AudioFormat;
use thiserror::Error;

/// Errors that can occur during audio format conversion
#[derive(Debug, Error)]
pub enum AudioFormatError {
    #[error("Unsupported sample format")]
    UnsupportedFormat,

    #[error("Invalid channel count: {0}")]
    InvalidChannelCount(u16),

    #[error("Invalid sample rate: {0}")]
    InvalidSampleRate(u32),

    #[error("Conversion error: {0}")]
    ConversionError(String),

    #[error("Buffer size mismatch: expected {expected}, got {got}")]
    BufferSizeMismatch { expected: usize, got: usize },
}

// Internal audio format for conversion
#[derive(Debug, Clone)]
struct InternalAudioFormat {
    sample_rate: u32,
    channels: u16,
    sample_format: InternalSampleFormat,
}

#[derive(Debug, Clone)]
enum InternalSampleFormat {
    Unknown,
    I16,
    I24,
    I32,
    F32,
    F64,
}

impl std::fmt::Display for InternalSampleFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InternalSampleFormat::Unknown => write!(f, "Unknown"),
            InternalSampleFormat::I16 => write!(f, "I16"),
            InternalSampleFormat::I24 => write!(f, "I24"),
            InternalSampleFormat::I32 => write!(f, "I32"),
            InternalSampleFormat::F32 => write!(f, "F32"),
            InternalSampleFormat::F64 => write!(f, "F64"),
        }
    }
}

impl From<&AudioFormat> for InternalAudioFormat {
    fn from(proto_format: &AudioFormat) -> Self {
        Self {
            sample_rate: proto_format.sample_rate,
            channels: proto_format.channels as u16,
            sample_format: match proto_format.sample_format {
                1 => InternalSampleFormat::I16,
                2 => InternalSampleFormat::I24,
                3 => InternalSampleFormat::I32,
                4 => InternalSampleFormat::F32,
                5 => InternalSampleFormat::F64,
                _ => InternalSampleFormat::Unknown,
            },
        }
    }
}

pub struct AudioConverter {
    target_format: InternalAudioFormat,
    resampler: Option<SincFixedIn<f32>>,
    input_sample_rate: u32,
    input_channels: u16,
    chunk_size: usize,
    channel_buffers: Vec<Vec<f32>>, // Per-channel input buffer
}

impl AudioConverter {
    pub fn new(
        input_format: &AudioFormat,
        target_format: &AudioFormat,
    ) -> Result<Self, AudioFormatError> {
        let input_format = InternalAudioFormat::from(input_format);
        let target_format = InternalAudioFormat::from(target_format);

        log::info!(
            "AudioConverter: Creating converter from {}Hz {}ch {} to {}Hz {}ch {}",
            input_format.sample_rate,
            input_format.channels,
            input_format.sample_format,
            target_format.sample_rate,
            target_format.channels,
            target_format.sample_format
        );

        let chunk_size = 1024; // Rubato default chunk size
        let resampler = if input_format.sample_rate != target_format.sample_rate {
            log::info!(
                "AudioConverter: Creating resampler from {}Hz to {}Hz (ratio: {})",
                input_format.sample_rate,
                target_format.sample_rate,
                target_format.sample_rate as f64 / input_format.sample_rate as f64
            );
            let ratio = target_format.sample_rate as f64 / input_format.sample_rate as f64;
            let params = SincInterpolationParameters {
                sinc_len: 32,
                f_cutoff: 0.95,
                interpolation: SincInterpolationType::Linear,
                oversampling_factor: 128,
                window: WindowFunction::BlackmanHarris2,
            };
            match SincFixedIn::<f32>::new(
                ratio,
                2.0, // fast mode
                params,
                chunk_size,                     // chunk_size first
                input_format.channels as usize, // channels second
            ) {
                Ok(resampler) => Some(resampler),
                Err(e) => {
                    return Err(AudioFormatError::ConversionError(format!(
                        "Failed to create resampler: {}",
                        e
                    )))
                }
            }
        } else {
            log::info!("AudioConverter: No resampling needed (same sample rate)");
            None
        };
        Ok(Self {
            target_format: target_format.clone(),
            resampler,
            input_sample_rate: input_format.sample_rate,
            input_channels: input_format.channels,
            chunk_size,
            channel_buffers: vec![Vec::new(); input_format.channels as usize],
        })
    }

    /// Convert F32 samples to the target sample format
    fn convert_sample_format(&self, samples: &[f32]) -> Result<Vec<u8>, AudioFormatError> {
        log::debug!(
            "AudioConverter: Converting {} F32 samples to {} format",
            samples.len(),
            self.target_format.sample_format
        );

        let result = match self.target_format.sample_format {
            InternalSampleFormat::I16 => {
                let mut bytes = Vec::with_capacity(samples.len() * 2);
                for &sample in samples {
                    let clamped = sample.max(-1.0).min(1.0);
                    let i16_sample = (clamped * 32767.0) as i16;
                    bytes.extend_from_slice(&i16_sample.to_le_bytes());
                }
                Ok(bytes)
            }
            InternalSampleFormat::I32 => {
                let mut bytes = Vec::with_capacity(samples.len() * 4);
                for &sample in samples {
                    let clamped = sample.max(-1.0).min(1.0);
                    let i32_sample = (clamped * 2147483647.0) as i32;
                    bytes.extend_from_slice(&i32_sample.to_le_bytes());
                }
                Ok(bytes)
            }
            InternalSampleFormat::F32 => {
                let mut bytes = Vec::with_capacity(samples.len() * 4);
                for &sample in samples {
                    bytes.extend_from_slice(&sample.to_le_bytes());
                }
                Ok(bytes)
            }
            InternalSampleFormat::F64 => {
                let mut bytes = Vec::with_capacity(samples.len() * 8);
                for &sample in samples {
                    let f64_sample = sample as f64;
                    bytes.extend_from_slice(&f64_sample.to_le_bytes());
                }
                Ok(bytes)
            }
            _ => Err(AudioFormatError::UnsupportedFormat),
        };

        if let Ok(ref bytes) = result {
            log::debug!(
                "AudioConverter: Sample format conversion complete: {} samples -> {} bytes",
                samples.len(),
                bytes.len()
            );
        }

        result
    }

    /// Feed input samples, return all available output samples as bytes in target format
    pub fn convert(&mut self, input: &[f32]) -> Result<Vec<u8>, AudioFormatError> {
        log::debug!(
            "AudioConverter: Received {} F32 samples ({} channels)",
            input.len(),
            self.input_channels
        );

        // Deinterleave into per-channel buffers (for original input format)
        for (i, sample) in input.iter().enumerate() {
            self.channel_buffers[i % self.input_channels as usize].push(*sample);
        }

        log::debug!(
            "AudioConverter: Buffer state after deinterleaving: {:?}",
            self.channel_buffers
                .iter()
                .map(|b| b.len())
                .collect::<Vec<_>>()
        );

        let mut output_samples: Vec<f32> = Vec::new();

        if let Some(resampler) = &mut self.resampler {
            // While all channels have enough samples for a chunk, process
            while self
                .channel_buffers
                .iter()
                .all(|ch| ch.len() >= self.chunk_size)
            {
                log::debug!(
                    "AudioConverter: Processing resampling chunk, buffer lens: {:?}",
                    self.channel_buffers
                        .iter()
                        .map(|b| b.len())
                        .collect::<Vec<_>>()
                );

                let mut input_chunks = Vec::with_capacity(self.input_channels as usize);
                for ch in 0..self.input_channels as usize {
                    input_chunks.push(self.channel_buffers[ch][..self.chunk_size].to_vec());
                }

                // Remove used samples
                for ch in 0..self.input_channels as usize {
                    self.channel_buffers[ch].drain(..self.chunk_size);
                }

                let out = resampler.process(&input_chunks, None).map_err(|e| {
                    AudioFormatError::ConversionError(format!("Resampling error: {}", e))
                })?;

                log::debug!(
                    "AudioConverter: Resampling complete: {} channels, {} samples per channel",
                    out.len(),
                    out[0].len()
                );

                // Interleave output (still in original format)
                for i in 0..out[0].len() {
                    for ch in 0..self.input_channels as usize {
                        output_samples.push(out[ch][i]);
                    }
                }
            }
        } else {
            // No resampling, just interleave and output in chunks
            while self
                .channel_buffers
                .iter()
                .all(|ch| ch.len() >= self.chunk_size)
            {
                log::debug!(
                    "AudioConverter: Processing no-resample chunk, buffer lens: {:?}",
                    self.channel_buffers
                        .iter()
                        .map(|b| b.len())
                        .collect::<Vec<_>>()
                );

                for i in 0..self.chunk_size {
                    for ch in 0..self.input_channels as usize {
                        output_samples.push(self.channel_buffers[ch][i]);
                    }
                }
                for ch in 0..self.input_channels as usize {
                    self.channel_buffers[ch].drain(..self.chunk_size);
                }
            }
        }

        log::debug!(
            "AudioConverter: After resampling/no-resample: {} samples",
            output_samples.len()
        );

        // Now do channel conversion if needed
        let final_output = if self.input_channels == 1 && self.target_format.channels == 2 {
            log::debug!(
                "AudioConverter: Converting mono to stereo: {} samples -> {} samples",
                output_samples.len(),
                output_samples.len() * 2
            );
            // Mono to stereo: duplicate each sample
            let mut stereo = Vec::with_capacity(output_samples.len() * 2);
            for &sample in &output_samples {
                stereo.push(sample);
                stereo.push(sample);
            }
            stereo
        } else {
            log::debug!(
                "AudioConverter: No channel conversion needed: {} samples",
                output_samples.len()
            );
            output_samples
        };

        // Convert to target sample format
        let result = self.convert_sample_format(&final_output);

        if let Ok(ref bytes) = result {
            log::debug!(
                "AudioConverter: Final output: {} bytes in {} format",
                bytes.len(),
                self.target_format.sample_format
            );
        }

        result
    }

    /// Flush any remaining samples (pad with zeros if needed)
    pub fn flush(&mut self) -> Result<Vec<u8>, AudioFormatError> {
        log::debug!(
            "AudioConverter: Flushing buffers: {:?}",
            self.channel_buffers
                .iter()
                .map(|b| b.len())
                .collect::<Vec<_>>()
        );

        let mut output_samples: Vec<f32> = Vec::new();

        // If all buffers are empty, nothing to flush
        if self.channel_buffers.iter().all(|b| b.is_empty()) {
            log::debug!("AudioConverter: No samples to flush");
            return Ok(Vec::new());
        }

        // Pad each channel buffer up to chunk_size
        for ch in 0..self.input_channels as usize {
            while self.channel_buffers[ch].len() < self.chunk_size {
                self.channel_buffers[ch].push(0.0);
            }
        }

        if let Some(resampler) = &mut self.resampler {
            log::debug!("AudioConverter: Flushing with resampling");
            // Prepare input chunks
            let mut input_chunks = Vec::with_capacity(self.input_channels as usize);
            for ch in 0..self.input_channels as usize {
                input_chunks.push(self.channel_buffers[ch][..self.chunk_size].to_vec());
            }
            let out = resampler.process(&input_chunks, None).map_err(|e| {
                AudioFormatError::ConversionError(format!("Resampling error: {}", e))
            })?;
            for i in 0..out[0].len() {
                for ch in 0..self.input_channels as usize {
                    output_samples.push(out[ch][i]);
                }
            }
        } else {
            log::debug!("AudioConverter: Flushing without resampling");
            for i in 0..self.chunk_size {
                for ch in 0..self.input_channels as usize {
                    output_samples.push(self.channel_buffers[ch][i]);
                }
            }
        }

        // Clear buffers
        for ch in 0..self.input_channels as usize {
            self.channel_buffers[ch].clear();
        }

        log::debug!(
            "AudioConverter: After flush processing: {} samples",
            output_samples.len()
        );

        // Now do channel conversion if needed
        let final_output = if self.input_channels == 1 && self.target_format.channels == 2 {
            log::debug!(
                "AudioConverter: Flush converting mono to stereo: {} samples -> {} samples",
                output_samples.len(),
                output_samples.len() * 2
            );
            // Mono to stereo: duplicate each sample
            let mut stereo = Vec::with_capacity(output_samples.len() * 2);
            for &sample in &output_samples {
                stereo.push(sample);
                stereo.push(sample);
            }
            stereo
        } else {
            log::debug!(
                "AudioConverter: Flush no channel conversion needed: {} samples",
                output_samples.len()
            );
            output_samples
        };

        // Convert to target sample format
        let result = self.convert_sample_format(&final_output);

        if let Ok(ref bytes) = result {
            log::debug!(
                "AudioConverter: Flush final output: {} bytes in {} format",
                bytes.len(),
                self.target_format.sample_format
            );
        }

        result
    }
}
