//! Voice Activity Detection using WebRTC VAD
//!
//! This module provides efficient voice activity detection to reduce CPU usage
//! by only running the expensive wakeword detection when speech is detected.

use crate::error::{EdgeError, Result};
use std::collections::VecDeque;
use strum::Display;
use webrtc_vad::{SampleRate, Vad, VadMode};

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
            mode: VADMode::Aggressive, // Skew towards false positives over false negatives
            sample_rate: VADSampleRate::Rate16kHz,
            frame_duration_ms: 20, // 20ms frames = 320 samples at 16kHz (good balance)
            speech_trigger_frames: 2, // 40ms of consecutive speech to trigger
            silence_stop_frames: 5, // 100ms of silence to stop
        }
    }
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

    /// Process i16 audio samples directly and return whether to run wakeword detection
    pub fn should_process_audio(&mut self, samples: &[i16]) -> Result<bool> {
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

    /// Reset VAD state
    pub fn reset(&mut self) {
        self.recent_decisions.clear();
        self.is_speech_active = false;
        self.audio_buffer.clear();
        log::info!("🔄 VAD: State reset");
    }
}
