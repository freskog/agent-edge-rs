use thiserror::Error;

#[derive(Error, Debug, Clone)]
pub enum AgentError {
    #[error("Audio error: {0}")]
    Audio(String),
    #[error("STT error: {0}")]
    STT(String),
    #[error("TTS error: {0}")]
    TTS(String),
    #[error("Wakeword error: {0}")]
    Wakeword(String),
    #[error("Configuration error: {0}")]
    Config(String),
    #[error("Unknown error: {0}")]
    Unknown(String),
    #[error("Base64 decode error: {0}")]
    Base64DecodeError(String),
    #[error("Invalid JSON: {0}")]
    InvalidJson(String),
    #[error("Missing field: {0}")]
    MissingField(String),
    #[error("Failed to save audio: {0}")]
    FailedToSaveAudio(String),
    #[error("MP3 decoding not implemented")]
    Mp3DecodingNotImplemented,
    #[error("Write error: {0}")]
    WriteError(String),
}

pub type Result<T> = std::result::Result<T, AgentError>;

impl From<anyhow::Error> for AgentError {
    fn from(err: anyhow::Error) -> Self {
        AgentError::Unknown(err.to_string())
    }
}

impl From<std::io::Error> for AgentError {
    fn from(err: std::io::Error) -> Self {
        AgentError::Audio(err.to_string())
    }
}
