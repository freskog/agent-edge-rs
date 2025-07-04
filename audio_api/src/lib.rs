pub mod audio_capture;
pub mod audio_sink;
pub mod audio_streamer;
pub mod config;
pub mod error;
pub mod types;

// Re-export common types
pub use audio_capture::{AudioCapture, AudioCaptureConfig};
pub use audio_sink::*;
pub use audio_streamer::events::AudioEvent;
pub use audio_streamer::{AudioChunk, AudioHub, AudioStreamer};
pub use config::*;
pub use error::{EdgeError, Result as EdgeResult};
pub use types::*;
