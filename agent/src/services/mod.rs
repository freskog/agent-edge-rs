pub mod llm;
pub mod stt;
pub mod tts;
pub mod types;

use self::types::LLMResponse;
use crate::error::AgentError;

/// Service trait for speech-to-text functionality
/// Fully blocking implementation
pub trait STTService: Send + Sync {
    /// Start background audio buffering from the audio service
    fn start_audio_buffering(&mut self) -> Result<(), AgentError>;

    /// Transcribe speech starting from wakeword detection
    /// Uses buffered context + new streaming audio until EOS
    fn transcribe_from_wakeword(&mut self) -> Result<String, AgentError>;

    /// Transcribe speech from provided audio chunks (new streaming approach)
    /// Used when audio chunks are provided directly from wakeword streaming
    fn transcribe_from_chunks(
        &self,
        audio_chunks: Vec<wakeword_protocol::protocol::AudioChunk>,
    ) -> Result<String, AgentError>;
}

/// Service trait for LLM processing (now blocking with ureq)
pub trait LLMService: Send + Sync {
    /// Process user transcript and return tool calls
    fn process(&self, transcript: String) -> Result<LLMResponse, AgentError>;
}

/// Service trait for text-to-speech functionality (now blocking with tungstenite)
pub trait TTSService: Send + Sync {
    /// Speak the given text
    fn speak(&self, text: String) -> Result<(), AgentError>;
}
