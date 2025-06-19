use thiserror::Error;

pub type Result<T> = std::result::Result<T, EdgeError>;

#[derive(Error, Debug)]
pub enum EdgeError {
    #[error("Audio error: {0}")]
    Audio(String),

    #[error("Model error: {0}")]
    Model(String),

    #[error("Model loading error: {0}")]
    ModelLoadError(String),

    #[error("Processing error: {0}")]
    ProcessingError(String),

    #[error("Invalid input: {0}")]
    InvalidInput(String),

    #[error("Detection error: {0}")]
    Detection(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("TensorFlow Lite error: {0}")]
    TfLite(String),

    #[error("Configuration error: {0}")]
    Config(String),
}
