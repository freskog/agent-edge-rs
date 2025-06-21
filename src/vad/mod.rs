//! Voice Activity Detection using WebRTC VAD
//!
//! This module provides efficient voice activity detection to reduce CPU usage
//! by only running the expensive wakeword detection when speech is detected.

use crate::error::{EdgeError, Result};
use std::collections::VecDeque;
use webrtc_vad::{SampleRate, Vad, VadMode};

/// Configuration for Voice Activity Detection
#[derive(Debug, Clone)]
pub struct VADConfig {
    /// VAD aggressiveness mode (0-3). Higher values = less sensitive
    /// 0 = Quality, 1 = Low Bitrate, 2 = Aggressive, 3 = Very Aggressive
    pub mode: u8,
    /// Sample rate (8000, 16000, 32000, or 48000 Hz)
    pub sample_rate: u32,
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
            mode: 3, // Very Aggressive mode - most conservative
            sample_rate: 16000,
            frame_duration_ms: 20, // 20ms frames = 320 samples at 16kHz (good balance)
            speech_trigger_frames: 2, // 40ms of consecutive speech to trigger
            silence_stop_frames: 5, // 100ms of silence to stop
        }
    }
}

/// WebRTC VAD wrapper with state management
pub struct WebRtcVAD {
    vad: Vad,
    config: VADConfig,
    frame_size: usize,
    recent_decisions: VecDeque<bool>,
    is_speech_active: bool,
    debug_frame_count: u64,
    // Audio buffer to accumulate samples for proper frame sizes
    audio_buffer: Vec<i16>,
}

impl WebRtcVAD {
    /// Create a new WebRTC VAD instance
    pub fn new(config: VADConfig) -> Result<Self> {
        // Convert mode to VadMode enum
        let vad_mode = match config.mode {
            0 => VadMode::Quality,
            1 => VadMode::LowBitrate,
            2 => VadMode::Aggressive,
            3 => VadMode::VeryAggressive,
            _ => {
                return Err(EdgeError::VADError(
                    "Invalid VAD mode. Must be 0-3".to_string(),
                ));
            }
        };

        // Convert sample rate to SampleRate enum
        let sample_rate = match config.sample_rate {
            8000 => SampleRate::Rate8kHz,
            16000 => SampleRate::Rate16kHz,
            32000 => SampleRate::Rate32kHz,
            48000 => SampleRate::Rate48kHz,
            _ => {
                return Err(EdgeError::VADError(
                    "Invalid sample rate. Must be 8000, 16000, 32000, or 48000".to_string(),
                ));
            }
        };

        // Validate frame duration
        if ![10, 20, 30].contains(&config.frame_duration_ms) {
            return Err(EdgeError::VADError(
                "Invalid frame duration. Must be 10, 20, or 30ms".to_string(),
            ));
        }

        // Calculate frame size in samples
        let frame_size = (config.sample_rate * config.frame_duration_ms / 1000) as usize;

        let vad = Vad::new_with_rate_and_mode(sample_rate, vad_mode);

        let mode_str = match config.mode {
            0 => "Quality",
            1 => "LowBitrate",
            2 => "Aggressive",
            3 => "VeryAggressive",
            _ => "Unknown",
        };

        println!(
            "🎤 WebRTC VAD ready ({} mode, {}ms frames = {} samples)",
            mode_str, config.frame_duration_ms, frame_size
        );

        Ok(Self {
            vad,
            config: config.clone(),
            frame_size,
            recent_decisions: VecDeque::with_capacity(
                config.speech_trigger_frames.max(config.silence_stop_frames),
            ),
            is_speech_active: false,
            debug_frame_count: 0,
            audio_buffer: Vec::new(),
        })
    }

    /// Process i16 audio samples directly and return whether to run wakeword detection
    ///
    /// This method works directly with i16 samples to avoid double conversion
    pub fn should_process_audio_i16(&mut self, samples: &[i16]) -> Result<bool> {
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

            self.debug_frame_count += 1;

            // Update state based on VAD decision
            self.update_vad_state(is_voice);

            if is_voice {
                any_speech_detected = true;
            }

            // Debug output every 100 frames (~2 seconds with 20ms frames)
            if self.debug_frame_count % 100 == 0 {
                println!(
                    "🔍 VAD Debug: frame #{} = {}, active: {}",
                    self.debug_frame_count, is_voice, self.is_speech_active
                );
            }
        }

        // Return true if we're currently in speech mode or detected speech in this batch
        Ok(self.is_speech_active || any_speech_detected)
    }

    /// Process f32 audio samples (legacy method for compatibility)
    pub fn should_process_audio(&mut self, samples: &[f32]) -> Result<bool> {
        // Convert f32 to i16 with proper scaling
        let samples_i16: Vec<i16> = samples
            .iter()
            .map(|&s| (s.clamp(-1.0, 1.0) * 32767.0) as i16)
            .collect();

        self.should_process_audio_i16(&samples_i16)
    }

    /// Update VAD state based on recent decisions
    fn update_vad_state(&mut self, is_voice: bool) {
        // Add to recent decisions
        self.recent_decisions.push_back(is_voice);
        if self.recent_decisions.len()
            > self
                .config
                .speech_trigger_frames
                .max(self.config.silence_stop_frames)
        {
            self.recent_decisions.pop_front();
        }

        // Check for speech start
        if !self.is_speech_active {
            let recent_speech_count = self
                .recent_decisions
                .iter()
                .rev()
                .take(self.config.speech_trigger_frames)
                .filter(|&&decision| decision)
                .count();

            if recent_speech_count >= self.config.speech_trigger_frames {
                self.is_speech_active = true;
                println!(
                    "🎤 VAD: Speech detected - starting wakeword processing ({}ms trigger)",
                    self.config.speech_trigger_frames as u32 * self.config.frame_duration_ms
                );
            }
        } else {
            // Check for speech end
            let recent_silence_count = self
                .recent_decisions
                .iter()
                .rev()
                .take(self.config.silence_stop_frames)
                .filter(|&&decision| !decision)
                .count();

            if recent_silence_count >= self.config.silence_stop_frames {
                self.is_speech_active = false;
                println!(
                    "🔇 VAD: Silence detected - stopping wakeword processing ({}ms silence)",
                    self.config.silence_stop_frames as u32 * self.config.frame_duration_ms
                );
            }
        }
    }

    /// Reset VAD state
    pub fn reset(&mut self) {
        self.recent_decisions.clear();
        self.is_speech_active = false;
        self.audio_buffer.clear();
        println!("🔄 VAD: State reset");
    }

    /// Get current speech activity status
    pub fn is_speech_active(&self) -> bool {
        self.is_speech_active
    }

    /// Get frame size in samples
    pub fn frame_size(&self) -> usize {
        self.frame_size
    }

    /// Update VAD configuration (requires recreation)
    pub fn update_config(&mut self, config: VADConfig) -> Result<()> {
        *self = Self::new(config)?;
        Ok(())
    }
}

/// Statistics for VAD performance tracking
pub struct VADStats {
    pub total_frames: u64,
    pub speech_frames: u64,
    pub processing_time_ms: u64,
    pub cpu_savings_percent: f32,
}

impl Default for VADStats {
    fn default() -> Self {
        Self {
            total_frames: 0,
            speech_frames: 0,
            processing_time_ms: 0,
            cpu_savings_percent: 0.0,
        }
    }
}

impl VADStats {
    pub fn update(&mut self, is_speech: bool, processing_time_ms: u64) {
        self.total_frames += 1;
        if is_speech {
            self.speech_frames += 1;
            self.processing_time_ms += processing_time_ms;
        }

        // Calculate CPU savings (time not spent on wakeword processing)
        if self.total_frames > 0 {
            let frames_skipped = self.total_frames - self.speech_frames;
            self.cpu_savings_percent = (frames_skipped as f32 / self.total_frames as f32) * 100.0;
        }
    }

    pub fn reset(&mut self) {
        self.total_frames = 0;
        self.speech_frames = 0;
        self.processing_time_ms = 0;
        self.cpu_savings_percent = 0.0;
    }
}
