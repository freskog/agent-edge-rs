//! Voice Activity Detection
//!
//! This module provides efficient voice activity detection to reduce CPU usage
//! by only running the expensive wakeword detection when speech is detected.
//!
//! Supports both WebRTC VAD (legacy) and Silero VAD (recommended for better accuracy).

use crate::error::{EdgeError, Result};
use std::collections::VecDeque;
use strum::Display;
use webrtc_vad::{SampleRate, Vad, VadMode};

// Silero VAD implementation
pub mod silero;
pub use silero::SileroVAD;

/// VAD implementation type
#[derive(Debug, Clone, Copy, Display)]
pub enum VADType {
    WebRTC,
    Silero,
}

/// VAD mode wrapper with automatic string conversion
#[derive(Debug, Clone, Copy, Display)]
pub enum VADMode {
    Quality,
    LowBitrate,
    Aggressive,
    VeryAggressive,
}

impl From<VADMode> for VadMode {
    fn from(mode: VADMode) -> Self {
        match mode {
            VADMode::Quality => VadMode::Quality,
            VADMode::LowBitrate => VadMode::LowBitrate,
            VADMode::Aggressive => VadMode::Aggressive,
            VADMode::VeryAggressive => VadMode::VeryAggressive,
        }
    }
}

/// Sample rate wrapper with automatic conversions
#[derive(Debug, Clone, Copy, Display)]
pub enum VADSampleRate {
    #[strum(serialize = "8kHz")]
    Rate8kHz = 8000,
    #[strum(serialize = "16kHz")]
    Rate16kHz = 16000,
    #[strum(serialize = "32kHz")]
    Rate32kHz = 32000,
    #[strum(serialize = "48kHz")]
    Rate48kHz = 48000,
}

impl From<VADSampleRate> for SampleRate {
    fn from(rate: VADSampleRate) -> Self {
        match rate {
            VADSampleRate::Rate8kHz => SampleRate::Rate8kHz,
            VADSampleRate::Rate16kHz => SampleRate::Rate16kHz,
            VADSampleRate::Rate32kHz => SampleRate::Rate32kHz,
            VADSampleRate::Rate48kHz => SampleRate::Rate48kHz,
        }
    }
}

impl From<VADSampleRate> for u32 {
    fn from(rate: VADSampleRate) -> Self {
        rate as u32
    }
}

/// Configuration for Voice Activity Detection
#[derive(Debug, Clone)]
pub struct VADConfig {
    /// VAD implementation type
    pub vad_type: VADType,
    /// VAD aggressiveness mode
    pub mode: VADMode,
    /// Sample rate
    pub sample_rate: VADSampleRate,
    /// Frame duration in milliseconds (10, 20, or 30ms)
    pub frame_duration_ms: u32,
    /// Number of consecutive speech frames needed to trigger
    pub speech_trigger_frames: usize,
    /// Number of consecutive silence frames needed to stop
    pub silence_stop_frames: usize,
}

impl Default for VADConfig {
    fn default() -> Self {
        Self {
            vad_type: VADType::WebRTC, // Default to WebRTC for better performance
            mode: VADMode::LowBitrate, // Less aggressive to reduce false positives
            sample_rate: VADSampleRate::Rate16kHz,
            frame_duration_ms: 20, // 20ms frames = 320 samples at 16kHz (good balance)
            speech_trigger_frames: 3, // 60ms of consecutive speech to trigger (more selective)
            silence_stop_frames: 15, // 300ms of silence to stop (faster cutoff after speech)
        }
    }
}

/// Unified VAD trait for different implementations
pub trait VAD {
    /// Returns true if the VAD thinks the audio chunk should be processed.
    fn should_process_audio(&mut self, audio: &[i16]) -> Result<bool>;

    /// Returns true if speech is considered active.
    fn is_speech_active(&self) -> bool;

    /// Resets the VAD's internal state.
    fn reset(&mut self);
}

/// WebRTC VAD wrapper with state management
pub struct WebRtcVAD {
    vad: Vad,
    frame_size: usize,
    speech_trigger_frames: usize,
    silence_stop_frames: usize,
    frame_duration_ms: u32,
    recent_decisions: VecDeque<bool>,
    is_speech_active: bool,
    // Audio buffer to accumulate samples for proper frame sizes
    audio_buffer: Vec<i16>,
}

impl WebRtcVAD {
    /// Create a new WebRTC VAD instance
    pub fn new(config: VADConfig) -> Result<Self> {
        // Validate frame duration
        if ![10, 20, 30].contains(&config.frame_duration_ms) {
            return Err(EdgeError::VADError(
                "Invalid frame duration. Must be 10, 20, or 30ms".to_string(),
            ));
        }

        // Calculate frame size in samples
        let sample_rate_hz: u32 = config.sample_rate.into();
        let frame_size = (sample_rate_hz * config.frame_duration_ms / 1000) as usize;

        let vad = Vad::new_with_rate_and_mode(config.sample_rate.into(), config.mode.into());

        log::info!(
            "WebRTC VAD ready ({} mode, {} @ {}ms frames = {} samples)",
            config.mode,
            config.sample_rate,
            config.frame_duration_ms,
            frame_size
        );

        Ok(Self {
            vad,
            frame_size,
            speech_trigger_frames: config.speech_trigger_frames,
            silence_stop_frames: config.silence_stop_frames,
            frame_duration_ms: config.frame_duration_ms,
            recent_decisions: VecDeque::with_capacity(
                config.speech_trigger_frames.max(config.silence_stop_frames),
            ),
            is_speech_active: false,
            audio_buffer: Vec::new(),
        })
    }
}

impl VAD for WebRtcVAD {
    /// Process i16 audio samples directly and return whether to run wakeword detection
    fn should_process_audio(&mut self, samples: &[i16]) -> Result<bool> {
        // Add samples to our buffer
        self.audio_buffer.extend_from_slice(samples);

        let mut any_speech_detected = false;

        // Process complete frames
        while self.audio_buffer.len() >= self.frame_size {
            // Extract one frame
            let frame: Vec<i16> = self.audio_buffer.drain(0..self.frame_size).collect();

            // Process frame with WebRTC VAD
            let is_voice = self
                .vad
                .is_voice_segment(&frame)
                .map_err(|_| EdgeError::VADError("Invalid frame length for VAD".to_string()))?;

            // Update state based on VAD decision
            self.update_vad_state(is_voice);

            if is_voice {
                any_speech_detected = true;
            }
        }

        // Return true if we're currently in speech mode or detected speech in this batch
        Ok(self.is_speech_active || any_speech_detected)
    }

    /// Reset VAD state
    fn reset(&mut self) {
        self.recent_decisions.clear();
        self.is_speech_active = false;
        self.audio_buffer.clear();
        log::info!("🔄 WebRTC VAD: State reset");
    }

    /// Returns true if speech is considered active.
    fn is_speech_active(&self) -> bool {
        self.is_speech_active
    }
}

impl WebRtcVAD {
    /// Update VAD state based on recent decisions
    fn update_vad_state(&mut self, is_voice: bool) {
        // Add to recent decisions
        self.recent_decisions.push_back(is_voice);
        if self.recent_decisions.len() > self.speech_trigger_frames.max(self.silence_stop_frames) {
            self.recent_decisions.pop_front();
        }

        // Check for speech start
        if !self.is_speech_active {
            let recent_speech_count = self
                .recent_decisions
                .iter()
                .rev()
                .take(self.speech_trigger_frames)
                .filter(|&&decision| decision)
                .count();

            if recent_speech_count >= self.speech_trigger_frames {
                self.is_speech_active = true;
                log::debug!(
                    "VAD: Speech detected - starting wakeword processing ({}ms trigger)",
                    self.speech_trigger_frames as u32 * self.frame_duration_ms
                );
            }
        } else {
            // Check for speech end
            let recent_silence_count = self
                .recent_decisions
                .iter()
                .rev()
                .take(self.silence_stop_frames)
                .filter(|&&decision| !decision)
                .count();

            if recent_silence_count >= self.silence_stop_frames {
                self.is_speech_active = false;
                log::debug!(
                    "🔇 VAD: Silence detected - stopping wakeword processing ({}ms silence)",
                    self.silence_stop_frames as u32 * self.frame_duration_ms
                );
            }
        }
    }
}

/// Factory function to create the appropriate VAD implementation
pub fn create_vad(config: VADConfig) -> Result<Box<dyn VAD>> {
    match config.vad_type {
        VADType::WebRTC => {
            let vad = WebRtcVAD::new(config)?;
            Ok(Box::new(vad))
        }
        VADType::Silero => {
            let vad = SileroVAD::new(config)?;
            Ok(Box::new(vad))
        }
    }
}
