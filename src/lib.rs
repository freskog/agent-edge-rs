//! The main library for the `agent-edge-rs` voice assistant.
//!
//! This library provides all the core components for building the edge agent,
//! including audio capture, VAD, wakeword detection, and STT streaming.

// Public modules, accessible to the binary and other consumers
pub mod audio_capture;
pub mod config;
pub mod detection;
pub mod error;
pub mod led_ring;
pub mod llm;
pub mod models;
pub mod speech_producer;
pub mod stt;
pub mod tts;
pub mod user_instruction;

// Re-export key types for convenience
pub use error::{EdgeError, Result};
pub use stt::{FireworksSTT, STTConfig};

/// Represents a chunk of audio data captured from the microphone.
///
/// This struct is made public to be shared between the audio capture loop
/// in the main binary and the STT streaming module in the library.
#[derive(Debug, Clone)]
pub struct AudioChunk {
    pub samples_i16: Vec<i16>,
    pub samples_f32: Vec<f32>,
    pub timestamp: std::time::Instant,
    pub should_process: bool, // VAD result included
}
