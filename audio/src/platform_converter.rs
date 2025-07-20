use crate::platform::{AudioPlatform, PlatformSampleFormat};
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PlatformConverterError {
    #[error("Resampling error: {0}")]
    ResamplingError(String),

    #[error("Invalid buffer size: expected {expected}, got {got}")]
    InvalidBufferSize { expected: usize, got: usize },

    #[error("Conversion not supported for this platform")]
    UnsupportedConversion,
}

/// Platform-specific audio converter that handles minimal conversions
pub struct PlatformConverter {
    platform: AudioPlatform,
    resampler: Option<SincFixedIn<f32>>,
    input_rate: u32,
    output_rate: u32,
    input_format: PlatformSampleFormat,
    output_format: PlatformSampleFormat,
    buffer: Vec<f32>,
}

impl PlatformConverter {
    /// Create a new platform-specific converter
    pub fn new(
        platform: AudioPlatform,
        input_rate: u32,
        output_rate: u32,
        input_format: PlatformSampleFormat,
        output_format: PlatformSampleFormat,
    ) -> Result<Self, PlatformConverterError> {
        log::info!(
            "🔄 Creating platform converter for {}: {}Hz {} -> {}Hz {}",
            platform,
            input_rate,
            input_format,
            output_rate,
            output_format
        );

        // Create resampler if needed
        let resampler = if input_rate != output_rate {
            let ratio = output_rate as f64 / input_rate as f64;
            let params = SincInterpolationParameters {
                sinc_len: 32,
                f_cutoff: 0.95,
                interpolation: SincInterpolationType::Linear,
                oversampling_factor: 128,
                window: WindowFunction::BlackmanHarris2,
            };

            let chunk_size = 1024;
            match SincFixedIn::<f32>::new(ratio, 2.0, params, chunk_size, 1) {
                Ok(resampler) => {
                    log::info!("🔄 Created resampler: ratio = {:.3}", ratio);
                    Some(resampler)
                }
                Err(e) => {
                    return Err(PlatformConverterError::ResamplingError(format!(
                        "Failed to create resampler: {}",
                        e
                    )));
                }
            }
        } else {
            log::info!("🔄 No resampling needed (same sample rate)");
            None
        };

        Ok(Self {
            platform,
            resampler,
            input_rate,
            output_rate,
            input_format,
            output_format,
            buffer: Vec::new(),
        })
    }

    /// Convert audio samples using platform-specific logic
    pub fn convert(&mut self, input: &[f32]) -> Result<Vec<u8>, PlatformConverterError> {
        if input.is_empty() {
            return Ok(Vec::new());
        }

        // Add input to buffer
        self.buffer.extend_from_slice(input);

        let mut output_samples = Vec::new();

        // Apply resampling if needed
        if let Some(ref mut resampler) = self.resampler {
            let chunk_size = 1024;

            // Process in chunks
            while self.buffer.len() >= chunk_size {
                let chunk: Vec<f32> = self.buffer.drain(..chunk_size).collect();
                let input_channels = vec![chunk];

                match resampler.process(&input_channels, None) {
                    Ok(output_channels) => {
                        output_samples.extend_from_slice(&output_channels[0]);
                    }
                    Err(e) => {
                        return Err(PlatformConverterError::ResamplingError(e.to_string()));
                    }
                }
            }
        } else {
            // No resampling needed, just use buffered samples
            output_samples.extend_from_slice(&self.buffer);
            self.buffer.clear();
        }

        // Convert sample format
        self.convert_samples_to_bytes(&output_samples)
    }

    /// Flush any remaining samples
    pub fn flush(&mut self) -> Result<Vec<u8>, PlatformConverterError> {
        if self.buffer.is_empty() {
            return Ok(Vec::new());
        }

        let mut output_samples = Vec::new();

        if let Some(ref mut resampler) = self.resampler {
            // Pad buffer to chunk size if needed
            let chunk_size = 1024;
            while self.buffer.len() < chunk_size {
                self.buffer.push(0.0);
            }

            let input_channels = vec![self.buffer.clone()];
            match resampler.process(&input_channels, None) {
                Ok(output_channels) => {
                    output_samples.extend_from_slice(&output_channels[0]);
                }
                Err(e) => {
                    return Err(PlatformConverterError::ResamplingError(e.to_string()));
                }
            }
        } else {
            output_samples.extend_from_slice(&self.buffer);
        }

        self.buffer.clear();
        self.convert_samples_to_bytes(&output_samples)
    }

    /// Convert f32 samples to bytes in the target format
    fn convert_samples_to_bytes(&self, samples: &[f32]) -> Result<Vec<u8>, PlatformConverterError> {
        match self.output_format {
            PlatformSampleFormat::I16 => {
                let mut bytes = Vec::with_capacity(samples.len() * 2);
                for &sample in samples {
                    let clamped = sample.clamp(-1.0, 1.0);
                    let i16_sample = (clamped * 32767.0) as i16;
                    bytes.extend_from_slice(&i16_sample.to_le_bytes());
                }
                Ok(bytes)
            }
            PlatformSampleFormat::F32 => {
                let mut bytes = Vec::with_capacity(samples.len() * 4);
                for &sample in samples {
                    bytes.extend_from_slice(&sample.to_le_bytes());
                }
                Ok(bytes)
            }
        }
    }
}

/// Create a converter for audio capture (device -> STT/Wakeword format)
pub fn create_capture_converter(
    platform: AudioPlatform,
    device_rate: u32,
) -> Result<PlatformConverter, PlatformConverterError> {
    let platform_config = platform.capture_config();
    let stt_format = platform.stt_format();

    PlatformConverter::new(
        platform,
        device_rate,
        stt_format.sample_rate,
        platform_config.preferred_format,
        stt_format.format,
    )
}

/// Create a converter for audio playback (TTS -> device format)
pub fn create_playback_converter(
    platform: AudioPlatform,
) -> Result<PlatformConverter, PlatformConverterError> {
    let tts_format = platform.tts_format();
    let playback_config = platform.playback_config();

    PlatformConverter::new(
        platform,
        tts_format.sample_rate,
        playback_config.sample_rate,
        tts_format.format,
        playback_config.format,
    )
}

/// Simple helper for mono->stereo conversion
pub fn mono_to_stereo(mono_samples: &[f32]) -> Vec<f32> {
    let mut stereo = Vec::with_capacity(mono_samples.len() * 2);
    for &sample in mono_samples {
        stereo.push(sample); // Left channel
        stereo.push(sample); // Right channel
    }
    stereo
}

/// Simple helper for f32->i16 conversion (useful for Pi platform)
pub fn f32_to_i16_bytes(samples: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(samples.len() * 2);
    for &sample in samples {
        let clamped = sample.clamp(-1.0, 1.0);
        let i16_sample = (clamped * 32767.0) as i16;
        bytes.extend_from_slice(&i16_sample.to_le_bytes());
    }
    bytes
}

/// Simple helper for i16->f32 conversion (useful for Mac platform)
pub fn i16_bytes_to_f32(bytes: &[u8]) -> Result<Vec<f32>, PlatformConverterError> {
    if bytes.len() % 2 != 0 {
        return Err(PlatformConverterError::InvalidBufferSize {
            expected: bytes.len() / 2 * 2,
            got: bytes.len(),
        });
    }

    let mut samples = Vec::with_capacity(bytes.len() / 2);
    for chunk in bytes.chunks_exact(2) {
        let i16_sample = i16::from_le_bytes([chunk[0], chunk[1]]);
        let f32_sample = i16_sample as f32 / 32768.0;
        samples.push(f32_sample);
    }
    Ok(samples)
}
