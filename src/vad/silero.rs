//! Silero VAD implementation
//!
//! This module provides voice activity detection using the Silero neural network,
//! which offers better accuracy than WebRTC VAD, especially for non-English languages
//! like Swedish.

use crate::error::Result;
use crate::vad::{VADConfig, VADSampleRate, VAD};
use std::collections::VecDeque;
use voice_activity_detector::{IteratorExt, LabeledAudio, VoiceActivityDetector};

/// Silero VAD wrapper using the proper voice_activity_detector streaming API
pub struct SileroVAD {
    vad: VoiceActivityDetector,
    threshold: f32,
    speech_trigger_chunks: usize,
    silence_stop_chunks: usize,
    recent_decisions: VecDeque<bool>,
    is_speech_active: bool,
    debug_counter: usize,
    sample_rate: u32,
}

impl SileroVAD {
    /// Create a new Silero VAD instance
    pub fn new(config: VADConfig) -> Result<Self> {
        // Convert sample rate to the format expected by voice_activity_detector
        let sample_rate = match config.sample_rate {
            VADSampleRate::Rate8kHz => 8000,
            VADSampleRate::Rate16kHz => 16000,
        };

        // Get chunk size in samples
        let chunk_size = config.chunk_size.samples();

        // Create the VAD with optimal settings
        let vad = VoiceActivityDetector::builder()
            .sample_rate(sample_rate)
            .chunk_size(chunk_size)
            .build()
            .map_err(|e| {
                crate::error::EdgeError::VADError(format!("Failed to create Silero VAD: {}", e))
            })?;

        log::info!(
            "Silero VAD initialized (sample_rate: {}Hz, chunk_size: {} samples, threshold: {}, speech_trigger: {} chunks, silence_stop: {} chunks)",
            sample_rate,
            chunk_size,
            config.threshold,
            config.speech_trigger_chunks,
            config.silence_stop_chunks
        );

        Ok(Self {
            vad,
            threshold: config.threshold,
            speech_trigger_chunks: config.speech_trigger_chunks,
            silence_stop_chunks: config.silence_stop_chunks,
            recent_decisions: VecDeque::with_capacity(
                config.speech_trigger_chunks.max(config.silence_stop_chunks),
            ),
            is_speech_active: false,
            debug_counter: 0,
            sample_rate,
        })
    }

    /// Update VAD state based on recent decisions
    fn update_vad_state(&mut self, is_voice: bool) {
        // Add to recent decisions
        self.recent_decisions.push_back(is_voice);
        if self.recent_decisions.len() > self.speech_trigger_chunks.max(self.silence_stop_chunks) {
            self.recent_decisions.pop_front();
        }

        // Check for speech start
        if !self.is_speech_active {
            let recent_speech_count = self
                .recent_decisions
                .iter()
                .rev()
                .take(self.speech_trigger_chunks)
                .filter(|&&decision| decision)
                .count();

            if recent_speech_count >= self.speech_trigger_chunks {
                self.is_speech_active = true;
                log::debug!(
                    "VAD: Speech detected - starting wakeword processing ({} chunks trigger)",
                    self.speech_trigger_chunks
                );
            }
        } else {
            // Check for speech end
            let recent_silence_count = self
                .recent_decisions
                .iter()
                .rev()
                .take(self.silence_stop_chunks)
                .filter(|&&decision| !decision)
                .count();

            if recent_silence_count >= self.silence_stop_chunks {
                self.is_speech_active = false;
                log::debug!(
                    "🔇 VAD: Silence detected - stopping wakeword processing ({} chunks silence)",
                    self.silence_stop_chunks
                );
            }
        }
    }
}

impl VAD for SileroVAD {
    /// Process i16 audio samples using the proper voice_activity_detector streaming API
    fn should_process_audio(&mut self, audio: &[i16]) -> Result<bool> {
        self.debug_counter += 1;

        // Use the voice_activity_detector's label method correctly:
        // Feed individual samples to the iterator, let the VAD handle internal buffering
        let labels: Vec<LabeledAudio<i16>> = audio
            .iter()
            .copied()
            .label(&mut self.vad, self.threshold, 0) // No padding for more precise detection
            .collect();

        // Count speech vs non-speech chunks
        let speech_count = labels
            .iter()
            .filter(|label| matches!(label, LabeledAudio::Speech(_)))
            .count();
        let total_count = labels.len();
        let has_speech = speech_count > 0;
        let speech_percentage = if total_count > 0 {
            (speech_count * 100) / total_count
        } else {
            0
        };

        // Update VAD state
        self.update_vad_state(has_speech);

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

        Ok(self.is_speech_active || has_speech)
    }

    /// Reset VAD state
    fn reset(&mut self) {
        // Create a new VAD instance to reset internal state
        match VoiceActivityDetector::builder()
            .sample_rate(self.sample_rate)
            .chunk_size(512_usize) // Default to small chunk size on reset
            .build()
        {
            Ok(new_vad) => {
                self.vad = new_vad;
                self.recent_decisions.clear();
                self.is_speech_active = false;
                self.debug_counter = 0;
                log::info!("🔄 Silero VAD: State reset");
            }
            Err(e) => {
                log::error!("Failed to reset Silero VAD: {}", e);
            }
        }
    }

    /// Returns true if speech is considered active.
    fn is_speech_active(&self) -> bool {
        self.is_speech_active
    }
}
