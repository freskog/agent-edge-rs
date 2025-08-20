use samplerate::{convert, ConverterType};

/// High-quality resampler using libsamplerate for converting 16kHz s16le audio to 48kHz s16le
pub struct SimpleResampler {
    input_rate: u32,
    output_rate: u32,
}

impl SimpleResampler {
    /// Create a new resampler for 16kHz → 48kHz conversion
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let input_rate = 16000;
        let output_rate = 48000;

        Ok(Self {
            input_rate,
            output_rate,
        })
    }

    /// Resample s16le bytes from 16kHz to 48kHz using high-quality libsamplerate
    /// Input: mono 16kHz s16le bytes
    /// Output: mono 48kHz s16le bytes  
    pub fn resample_s16le(
        &mut self,
        input_bytes: &[u8],
    ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        if input_bytes.len() % 2 != 0 {
            return Err("Input must be valid s16le (even number of bytes)".into());
        }

        // Convert s16le bytes to f32 samples for libsamplerate
        let mut input_samples = Vec::with_capacity(input_bytes.len() / 2);
        for chunk in input_bytes.chunks_exact(2) {
            let i16_sample = i16::from_le_bytes([chunk[0], chunk[1]]);
            let f32_sample = i16_sample as f32 / 32768.0; // Convert to [-1.0, 1.0]
            input_samples.push(f32_sample);
        }

        // Use libsamplerate for high-quality resampling with anti-aliasing
        // SincBestQuality provides 145dB SNR and excellent anti-aliasing
        let resampled_samples = convert(
            self.input_rate,
            self.output_rate,
            1, // mono (1 channel)
            ConverterType::SincBestQuality,
            &input_samples,
        )?;

        // Convert back to s16le bytes
        let mut output_bytes = Vec::with_capacity(resampled_samples.len() * 2);
        for sample in resampled_samples {
            let clamped = sample.clamp(-1.0, 1.0);
            let i16_sample = (clamped * 32767.0) as i16;
            output_bytes.extend_from_slice(&i16_sample.to_le_bytes());
        }

        Ok(output_bytes)
    }

    /// Flush - libsamplerate handles this internally
    pub fn flush(&mut self) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        // libsamplerate's convert() function is stateless, so no buffering to flush
        Ok(Vec::new())
    }

    /// Get the expected output size for a given input size
    /// This is approximate due to resampling ratios
    pub fn expected_output_size(&self, input_sample_count: usize) -> usize {
        // 48000/16000 = 3.0 ratio (exact)
        let ratio = self.output_rate as f64 / self.input_rate as f64;
        (input_sample_count as f64 * ratio) as usize
    }
}
