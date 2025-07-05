//! Error types for OpenWakeWord Rust implementation

use thiserror::Error;

/// Result type alias for OpenWakeWord operations
pub type Result<T> = std::result::Result<T, OpenWakeWordError>;

/// Error types for OpenWakeWord operations
#[derive(Error, Debug)]
pub enum OpenWakeWordError {
    #[error("Model loading error: {0}")]
    ModelLoadError(String),

    #[error("Invalid input: {0}")]
    InvalidInput(String),

    #[error("Processing error: {0}")]
    ProcessingError(String),

    #[error("Configuration error: {0}")]
    ConfigurationError(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("TensorFlow Lite error: {0}")]
    TfliteError(String),
}

// Compatibility alias with the old error type name
pub type EdgeError = OpenWakeWordError;
