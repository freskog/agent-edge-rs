use thiserror::Error;

#[derive(Error, Debug)]
pub enum AgentError {
    #[error("STT error: {0}")]
    STT(String),

    #[error("LLM error: {0}")]
    LLM(String),

    #[error("TTS error: {0}")]
    TTS(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Audio error: {0}")]
    Audio(String),

    #[error("Network error: {0}")]
    Network(String),

    #[error("Wakeword error: {0}")]
    Wakeword(String),

    #[error("General error: {0}")]
    General(String),
}

pub type Result<T> = std::result::Result<T, AgentError>;

impl From<anyhow::Error> for AgentError {
    fn from(err: anyhow::Error) -> Self {
        AgentError::General(err.to_string())
    }
}

impl From<std::io::Error> for AgentError {
    fn from(err: std::io::Error) -> Self {
        AgentError::Audio(err.to_string())
    }
}
