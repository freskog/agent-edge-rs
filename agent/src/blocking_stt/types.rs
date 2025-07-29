use std::time::Instant;

/// Events that represent the state of speech detection
#[derive(Debug, Clone, PartialEq)]
pub enum SpeechEvent {
    /// User started speaking
    SpeechStarted,
    /// User is continuing to speak
    Speech,
    /// User stopped speaking (End of Speech detected)
    SpeechStopped,
    /// Audio present but no speech detected
    NoSpeech,
}

/// Specific error types for STT operations with distinct timeout handling
#[derive(Debug)]
pub enum STTError {
    /// Overall 60s timeout exceeded for entire transcription
    EmergencyTimeout,
    /// No audio received from server for 3+ seconds
    AudioTimeout,
    /// 4+ seconds of consecutive NoSpeech events
    NoSpeechTimeout,
    /// Audio read/connection errors
    AudioError(String),
    /// WebSocket communication errors
    WebSocketError(String),
    /// VAD processing errors
    VadError(String),
}

impl std::fmt::Display for STTError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            STTError::EmergencyTimeout => {
                write!(f, "Emergency timeout: transcription exceeded 60 seconds")
            }
            STTError::AudioTimeout => write!(f, "Audio timeout: no audio received for 3+ seconds"),
            STTError::NoSpeechTimeout => {
                write!(f, "No speech timeout: 4+ seconds without speech detected")
            }
            STTError::AudioError(msg) => write!(f, "Audio error: {}", msg),
            STTError::WebSocketError(msg) => write!(f, "WebSocket error: {}", msg),
            STTError::VadError(msg) => write!(f, "VAD error: {}", msg),
        }
    }
}

impl std::error::Error for STTError {}

/// A chunk of raw audio data with timing and speech state
#[derive(Debug, Clone)]
pub struct RawChunk {
    /// Raw s16le audio data (2 bytes per sample at 16kHz)
    pub data: Vec<u8>,
    /// When this chunk was captured
    pub timestamp: Instant,
    /// Speech detection state for this chunk
    pub event: SpeechEvent,
}

impl RawChunk {
    pub fn new(data: Vec<u8>, timestamp: Instant, event: SpeechEvent) -> Self {
        Self {
            data,
            timestamp,
            event,
        }
    }

    /// Get the number of audio samples in this chunk
    pub fn sample_count(&self) -> usize {
        self.data.len() / 2 // 2 bytes per i16 sample
    }

    /// Get the duration of this chunk in milliseconds (assuming 16kHz)
    pub fn duration_ms(&self) -> f32 {
        self.sample_count() as f32 / 16.0 // 16kHz sample rate
    }
}

/// Statistics for monitoring STT performance
#[derive(Debug, Default, Clone)]
pub struct STTStats {
    pub chunks_captured: usize,
    pub chunks_sent: usize,
    pub chunks_dropped: usize,
    pub bytes_sent: usize,
    pub transcription_start: Option<Instant>,
    pub transcription_end: Option<Instant>,
}

impl STTStats {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn log_summary(&self) {
        if let (Some(start), Some(end)) = (self.transcription_start, self.transcription_end) {
            let duration_ms = end.duration_since(start).as_millis();
            log::info!(
                "📊 STT Stats: {}/{} chunks sent ({} dropped), {} bytes, {}ms total",
                self.chunks_sent,
                self.chunks_captured,
                self.chunks_dropped,
                self.bytes_sent,
                duration_ms
            );
        }
    }
}
