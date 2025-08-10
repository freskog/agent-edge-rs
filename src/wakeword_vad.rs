use crate::wakeword_error::VadError;
use log::{debug, info};
use voice_activity_detector::VoiceActivityDetector;

/// Configuration for Voice Activity Detection
#[derive(Debug, Clone)]
pub struct VadConfig {
    /// Sample rate (should match audio chunks)
    pub sample_rate: u32,
    /// Chunk size for VAD processing (must be 512 for Silero VAD)
    pub chunk_size: usize,
    /// Speech detection threshold (0.0 to 1.0)
    pub speech_threshold: f32,
}

impl Default for VadConfig {
    fn default() -> Self {
        Self {
            sample_rate: 16000,    // 16kHz sample rate
            chunk_size: 512,       // 32ms chunks at 16kHz (required by Silero VAD)
            speech_threshold: 0.5, // Default threshold for speech detection
        }
    }
}

/// Voice Activity Detector with buffering for 1280→512 sample processing
pub struct VadProcessor {
    detector: VoiceActivityDetector,
    config: VadConfig,
    remainder_buffer: Vec<f32>, // Buffer for samples that don't fit in 512-sample chunks
}

impl VadProcessor {
    /// Create a new VAD processor with the given configuration
    pub fn new(config: VadConfig) -> Result<Self, VadError> {
        // Create the VAD detector using the builder pattern
        let detector = VoiceActivityDetector::builder()
            .chunk_size(config.chunk_size)
            .sample_rate(config.sample_rate as i64)
            .build()
            .map_err(|e| VadError::InitializationError(e.to_string()))?;

        // Calculate chunk duration based on chunk size and sample rate
        let chunk_duration_ms = (config.chunk_size as u64 * 1000) / config.sample_rate as u64;

        info!(
            "🎤 VAD initialized: chunk_size={}, sample_rate={}Hz, chunk_duration={}ms",
            config.chunk_size, config.sample_rate, chunk_duration_ms
        );

        Ok(Self {
            detector,
            config,
            remainder_buffer: Vec::new(),
        })
    }

    /// Analyze a 1280-sample audio chunk and return whether any speech was detected
    ///
    /// This method buffers samples across chunk boundaries to ensure all audio is processed
    /// by the VAD in proper 512-sample windows with no overlap and no data loss.
    ///
    /// # Arguments
    /// * `audio_data` - Raw audio bytes (1280 samples * 2 bytes = 2560 bytes)
    ///
    /// # Returns
    /// * `bool` - true if any speech was detected in this chunk, false otherwise
    pub fn analyze_chunk(&mut self, audio_data: &[u8]) -> Result<bool, VadError> {
        // Convert audio data to f32 samples for VAD processing
        let new_samples: Vec<f32> = audio_data
            .chunks_exact(2)
            .map(|chunk| {
                let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
                sample as f32 / 32768.0 // Convert to [-1.0, 1.0] range
            })
            .collect();

        debug!(
            "🎤 VAD analyze_chunk: {} new samples, {} buffered samples",
            new_samples.len(),
            self.remainder_buffer.len()
        );

        // Sanity check: we expect 1280 samples (2560 bytes / 2)
        if new_samples.len() != 1280 {
            debug!(
                "⚠️ VAD unexpected chunk size: got {} samples, expected 1280",
                new_samples.len()
            );
        }

        let mut any_speech = false;
        let mut processed_offset = 0;

        // If we have remainder from previous chunk, process it first
        if !self.remainder_buffer.is_empty() {
            // Calculate how many samples we need to complete a 512-sample chunk
            let samples_needed = 512 - self.remainder_buffer.len();

            // Check if we have enough new samples
            if new_samples.len() >= samples_needed {
                // We have enough samples to complete a full 512-sample chunk
                let mut combined = self.remainder_buffer.clone();
                combined.extend(&new_samples[0..samples_needed]);

                debug!(
                    "🎤 VAD processing combined chunk: {} remainder + {} new = 512 samples",
                    self.remainder_buffer.len(),
                    samples_needed
                );

                // Process the combined 512-sample chunk
                let speech_prob = self.detector.predict(combined.iter().copied());
                let has_speech = speech_prob >= self.config.speech_threshold;

                debug!(
                    "🎤 VAD combined chunk: speech_prob={:.3}, has_speech={}",
                    speech_prob, has_speech
                );

                if has_speech {
                    any_speech = true;
                }

                processed_offset = samples_needed; // Skip samples we just processed
                self.remainder_buffer.clear(); // Buffer is now empty
            } else {
                // Not enough new samples to complete a chunk, just add them to the buffer
                self.remainder_buffer.extend(&new_samples);
                debug!(
                    "🎤 VAD insufficient samples: {} remainder + {} new = {} total (need 512)",
                    self.remainder_buffer.len() - new_samples.len(),
                    new_samples.len(),
                    self.remainder_buffer.len()
                );

                // All samples are now in the buffer, nothing more to process
                return Ok(any_speech);
            }
        }

        // Process remaining complete 512-sample chunks from new samples
        let remaining_samples = &new_samples[processed_offset..];
        for (i, chunk_512) in remaining_samples.chunks_exact(512).enumerate() {
            let speech_prob = self.detector.predict(chunk_512.iter().copied());
            let has_speech = speech_prob >= self.config.speech_threshold;

            debug!(
                "🎤 VAD chunk {}: speech_prob={:.3}, has_speech={}",
                i, speech_prob, has_speech
            );

            if has_speech {
                any_speech = true;
            }
        }

        // Save any remainder for next chunk
        let remainder = remaining_samples.chunks_exact(512).remainder();
        if !remainder.is_empty() {
            self.remainder_buffer = remainder.to_vec();
            debug!(
                "🎤 VAD buffering {} samples for next chunk",
                remainder.len()
            );
        }

        debug!(
            "🎤 VAD result: any_speech={}, processed {} total samples",
            any_speech,
            new_samples.len()
        );

        Ok(any_speech)
    }

    /// Reset the VAD processor state (clear remainder buffer)
    pub fn reset(&mut self) {
        self.remainder_buffer.clear();
        debug!("🎤 VAD processor state reset - remainder buffer cleared");
    }

    /// Get current buffer state for debugging
    pub fn buffer_samples(&self) -> usize {
        self.remainder_buffer.len()
    }
}
