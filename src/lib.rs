pub mod audio_capture;
pub mod config;
pub mod detection;
pub mod error;
pub mod led_ring;
pub mod llm;
pub mod models;
pub mod stt;
pub mod tts;
pub mod vad;

pub use error::{EdgeError, Result};
