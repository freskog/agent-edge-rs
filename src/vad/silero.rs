//! Silero VAD implementation
//!
//! This module provides voice activity detection using the Silero neural network,
//! which offers better accuracy than WebRTC VAD, especially for non-English languages
//! like Swedish.

use crate::error::Result;
use crate::vad::{VAD, VADConfig};
use voice_activity_detector::{IteratorExt, LabeledAudio, VoiceActivityDetector};

/// Silero VAD wrapper using the proper voice_activity_detector streaming API
pub struct SileroVAD {
    vad: VoiceActivityDetector,
    threshold: f32,
    padding_chunks: usize,
    processed_samples: usize,
    debug_counter: usize,
}

impl SileroVAD {
    /// Create a new Silero VAD instance
    pub fn new(config: VADConfig) -> Result<Self> {
        // Convert sample rate to the format expected by voice_activity_detector
        let sample_rate = match config.sample_rate {
            crate::vad::VADSampleRate::Rate8kHz => 8000,
            crate::vad::VADSampleRate::Rate16kHz => 16000,
            crate::vad::VADSampleRate::Rate32kHz => 16000, // Silero only supports 8kHz and 16kHz
            crate::vad::VADSampleRate::Rate48kHz => 16000, // Silero only supports 8kHz and 16kHz
        };

        // Choose optimal chunk size based on sample rate
        // From voice_activity_detector docs:
        // - 8kHz: 256, 512, 768 samples
        // - 16kHz: 512, 768, 1024 samples
        let chunk_size = if sample_rate == 8000 { 512usize } else { 512usize };

        // Create the VAD with optimal settings
        let vad = VoiceActivityDetector::builder()
            .sample_rate(sample_rate)
            .chunk_size(chunk_size)
            .build()
            .map_err(|e| crate::error::EdgeError::VADError(format!("Failed to create Silero VAD: {}", e)))?;

        // Use the standard Silero VAD threshold for testing
        let threshold = 0.5;

        // Use minimal padding to be more responsive
        let padding_chunks = 0; // No padding for more precise detection

        log::info!(
            "Silero VAD initialized (sample_rate: {}Hz, chunk_size: {} samples, threshold: {}, padding: {} chunks)",
            sample_rate,
            chunk_size,
            threshold,
            padding_chunks
        );

        Ok(Self {
            vad,
            threshold,
            padding_chunks,
            processed_samples: 0,
            debug_counter: 0,
        })
    }
}

impl VAD for SileroVAD {
    /// Process i16 audio samples using the proper voice_activity_detector streaming API
    fn should_process_audio(&mut self, audio: &[i16]) -> Result<bool> {
        self.processed_samples += audio.len();
        self.debug_counter += 1;

        // Use the voice_activity_detector's label method correctly:
        // Feed individual samples to the iterator, let the VAD handle internal buffering
        let labels: Vec<LabeledAudio<i16>> = audio
            .iter()
            .copied()
            .label(&mut self.vad, self.threshold, self.padding_chunks)
            .collect();

        // Count speech vs non-speech chunks
        let speech_count = labels.iter().filter(|label| matches!(label, LabeledAudio::Speech(_))).count();
        let total_count = labels.len();
        let has_speech = speech_count > 0;
        let speech_percentage = if total_count > 0 { (speech_count * 100) / total_count } else { 0 };

        // More frequent debug logging to understand the pattern
        if self.debug_counter % 10 == 0 || has_speech {
            log::debug!(
                "Silero VAD: {} samples -> {} speech / {} total labels ({}%) - {}",
                audio.len(),
                speech_count,
                total_count,
                speech_percentage,
                if has_speech { "SPEECH" } else { "silence" }
            );
        }

        // Log a summary every 50 chunks to see the overall pattern
        if self.debug_counter % 50 == 0 {
            log::info!(
                "Silero VAD: Processed {} chunks, threshold: {}, last result: {}% speech",
                self.debug_counter,
                self.threshold,
                speech_percentage
            );
        }

        Ok(has_speech)
    }

    /// Reset VAD state
    fn reset(&mut self) {
        // Create a new VAD instance to reset internal state
        // This is necessary because the voice_activity_detector doesn't expose a reset method
        let sample_rate = 16000; // We know this from our initialization
        let chunk_size = 512usize;
        
        match VoiceActivityDetector::builder()
            .sample_rate(sample_rate)
            .chunk_size(chunk_size)
            .build()
        {
            Ok(new_vad) => {
                self.vad = new_vad;
                self.processed_samples = 0;
                self.debug_counter = 0;
                log::info!("🔄 Silero VAD: State reset");
            }
            Err(e) => {
                log::error!("Failed to reset Silero VAD: {}", e);
            }
        }
    }

    /// For the streaming approach, we don't maintain persistent speech state
    /// The label method handles all the state management internally
    fn is_speech_active(&self) -> bool {
        // The voice_activity_detector handles state internally
        // We rely on should_process_audio for all decisions
        false
    }
}
