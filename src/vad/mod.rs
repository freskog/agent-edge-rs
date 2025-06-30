//! Voice Activity Detection (VAD) Module
//!
//! This module provides voice activity detection capabilities using the Silero neural network model.
//! Silero VAD is optimized for 16kHz audio and supports chunk sizes of 512, 768, or 1024 samples.

use crate::error::Result;
use strum::{Display, EnumString};

pub mod silero;
use silero::SileroVAD;

/// Sample rates supported by Silero VAD
#[derive(Debug, Clone, Copy, PartialEq, Eq, EnumString, Display)]
pub enum VADSampleRate {
    #[strum(serialize = "8kHz")]
    Rate8kHz = 8000,
    #[strum(serialize = "16kHz")]
    Rate16kHz = 16000,
}

impl From<VADSampleRate> for u32 {
    fn from(rate: VADSampleRate) -> Self {
        rate as u32
    }
}

/// Chunk sizes supported by Silero VAD at 16kHz
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkSize {
    /// 512 samples (32ms at 16kHz) - Recommended for low latency
    Small,
    /// 768 samples (48ms at 16kHz) - Balanced
    Medium,
    /// 1024 samples (64ms at 16kHz) - Better accuracy
    Large,
}

impl ChunkSize {
    /// Get the chunk size in samples
    pub fn samples(&self) -> usize {
        match self {
            ChunkSize::Small => 512,
            ChunkSize::Medium => 768,
            ChunkSize::Large => 1024,
        }
    }
}

/// Configuration for Silero Voice Activity Detection
#[derive(Debug, Clone)]
pub struct VADConfig {
    /// Sample rate (8kHz or 16kHz)
    pub sample_rate: VADSampleRate,
    /// Chunk size in samples
    pub chunk_size: ChunkSize,
    /// Speech probability threshold (0.0-1.0), default 0.5
    pub threshold: f32,
    /// Number of consecutive speech chunks needed to trigger
    pub speech_trigger_chunks: usize,
    /// Number of consecutive silence chunks needed to stop
    pub silence_stop_chunks: usize,
}

impl Default for VADConfig {
    fn default() -> Self {
        Self {
            sample_rate: VADSampleRate::Rate16kHz,
            chunk_size: ChunkSize::Small, // 512 samples = 32ms at 16kHz
            threshold: 0.5,
            speech_trigger_chunks: 2, // 64ms of speech to trigger
            silence_stop_chunks: 8,   // 256ms of silence to stop
        }
    }
}

/// Unified VAD trait for different implementations
pub trait VAD: Send {
    /// Returns true if the VAD thinks the audio chunk should be processed.
    fn should_process_audio(&mut self, audio: &[i16]) -> Result<bool>;

    /// Returns true if speech is considered active.
    fn is_speech_active(&self) -> bool;

    /// Resets the VAD's internal state.
    fn reset(&mut self);
}

/// Factory function to create Silero VAD
pub fn create_vad(config: VADConfig) -> Result<Box<dyn VAD + Send>> {
    let vad = SileroVAD::new(config)?;
    Ok(Box::new(vad))
}
