//! Speech events and chunk definitions for the speech producer

use std::time::Instant;

/// Speech events that can occur during audio processing
#[derive(Debug, Clone, PartialEq)]
pub enum SpeechEvent {
    /// First speech chunk after silence - triggers speech processing
    StartedSpeaking,
    /// Ongoing speech chunk - continues speech processing  
    Speaking,
    /// First silence chunk after speech - signals end of speech
    StoppedSpeaking,
}

/// A chunk of audio with speech event information
#[derive(Debug, Clone)]
pub struct SpeechChunk {
    /// Audio samples in f32 format (used by downstream processing)
    /// Always 1280 samples for downstream processing
    pub samples_f32: [f32; 1280],
    /// Timestamp when the chunk was captured
    pub timestamp: Instant,
    /// The speech event for this chunk
    pub speech_event: SpeechEvent,
}

impl SpeechChunk {
    /// Create a new speech chunk
    pub fn new(samples_f32: [f32; 1280], timestamp: Instant, speech_event: SpeechEvent) -> Self {
        Self {
            samples_f32,
            timestamp,
            speech_event,
        }
    }

    /// Returns true if this chunk should be processed by downstream components
    /// (StartedSpeaking and Speaking events should be processed)
    pub fn should_process(&self) -> bool {
        matches!(
            self.speech_event,
            SpeechEvent::StartedSpeaking | SpeechEvent::Speaking
        )
    }

    /// Returns true if this is the start of a speech segment
    pub fn is_speech_start(&self) -> bool {
        matches!(self.speech_event, SpeechEvent::StartedSpeaking)
    }

    /// Returns true if this signals the end of a speech segment
    pub fn is_speech_end(&self) -> bool {
        matches!(self.speech_event, SpeechEvent::StoppedSpeaking)
    }

    /// Get the number of samples in this chunk
    pub fn len(&self) -> usize {
        self.samples_f32.len()
    }

    /// Returns true if the chunk is empty
    pub fn is_empty(&self) -> bool {
        false // Fixed-size array is never empty
    }

    /// Get the duration of this chunk in milliseconds (assuming 16kHz sample rate)
    pub fn duration_ms(&self) -> f32 {
        (self.samples_f32.len() as f32 / 16000.0) * 1000.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_speech_chunk_creation() {
        let samples_f32 = [0.0; 1280];
        let timestamp = Instant::now();
        let chunk = SpeechChunk::new(samples_f32, timestamp, SpeechEvent::StartedSpeaking);

        assert_eq!(chunk.samples_f32.len(), 1280);
        assert_eq!(chunk.speech_event, SpeechEvent::StartedSpeaking);
        assert!(!chunk.is_empty());
    }

    #[test]
    fn test_should_process() {
        let timestamp = Instant::now();
        let samples = [0.0; 1280];

        let started = SpeechChunk::new(samples, timestamp, SpeechEvent::StartedSpeaking);
        assert!(started.should_process());
        assert!(started.is_speech_start());
        assert!(!started.is_speech_end());

        let speaking = SpeechChunk::new(samples, timestamp, SpeechEvent::Speaking);
        assert!(speaking.should_process());
        assert!(!speaking.is_speech_start());
        assert!(!speaking.is_speech_end());

        let stopped = SpeechChunk::new(samples, timestamp, SpeechEvent::StoppedSpeaking);
        assert!(!stopped.should_process());
        assert!(!stopped.is_speech_start());
        assert!(stopped.is_speech_end());
    }

    #[test]
    fn test_duration_calculation() {
        let samples = [0.0; 1280]; // 1280 samples
        let chunk = SpeechChunk::new(samples, Instant::now(), SpeechEvent::Speaking);

        // 1280 samples at 16kHz = 80ms
        assert!((chunk.duration_ms() - 80.0).abs() < 0.1);
    }
}
