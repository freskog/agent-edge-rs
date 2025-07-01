use crate::audio_capture::AudioCaptureError;
use crate::config::ConfigError;
use crate::stt::STTError;
use std::io;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, EdgeError>;

#[derive(Error, Debug)]
pub enum EdgeError {
    #[error("Audio capture error: {0}")]
    Audio(String),

    #[error("Audio capture error: {0}")]
    AudioCapture(AudioCaptureError),

    #[error("Configuration error: {0}")]
    Config(ConfigError),

    #[error("STT error: {0}")]
    STT(STTError),

    #[error("Model loading error: {0}")]
    ModelLoadError(String),

    #[error("Processing error: {0}")]
    ProcessingError(String),

    #[error("Detection pipeline error: {0}")]
    Pipeline(String),

    #[error("Invalid input: {0}")]
    InvalidInput(String),

    #[error("VAD error: {0}")]
    VADError(String),

    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Unknown error: {0}")]
    Unknown(String),
}

// Add conversion from ConfigError to EdgeError
impl From<crate::config::ConfigError> for EdgeError {
    fn from(err: crate::config::ConfigError) -> Self {
        EdgeError::Config(err)
    }
}

// Add conversion from AudioCaptureError to EdgeError
impl From<crate::audio_capture::AudioCaptureError> for EdgeError {
    fn from(err: crate::audio_capture::AudioCaptureError) -> Self {
        EdgeError::AudioCapture(err)
    }
}

// Add conversion from STTError to EdgeError
impl From<crate::stt::STTError> for EdgeError {
    fn from(err: crate::stt::STTError) -> Self {
        EdgeError::STT(err)
    }
}
