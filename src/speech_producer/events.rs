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
    /// Only populated when speech is detected
    pub samples_f32: Vec<f32>,
    /// Timestamp when the chunk was captured
    pub timestamp: Instant,
    /// The speech event for this chunk
    pub speech_event: SpeechEvent,
}

impl SpeechChunk {
    /// Create a new speech chunk
    pub fn new(samples_f32: Vec<f32>, timestamp: Instant, speech_event: SpeechEvent) -> Self {
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
        self.samples_f32.is_empty()
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
        let samples = vec![0.1, 0.2, 0.3, 0.4];
        let timestamp = Instant::now();
        let chunk = SpeechChunk::new(
            samples
                .clone()
                .into_iter()
                .map(|x| (x * 32767.0) as i16)
                .collect(),
            timestamp,
            SpeechEvent::StartedSpeaking,
        );

        assert_eq!(
            chunk.samples_i16,
            samples.into_iter().map(|x| (x * 32767.0) as i16).collect()
        );
        assert_eq!(chunk.speech_event, SpeechEvent::StartedSpeaking);
        assert_eq!(chunk.len(), 4);
        assert!(!chunk.is_empty());
    }

    #[test]
    fn test_should_process() {
        let timestamp = Instant::now();
        let samples = vec![0.0; 256];

        let started = SpeechChunk::new(
            samples
                .clone()
                .into_iter()
                .map(|x| (x * 32767.0) as i16)
                .collect(),
            timestamp,
            SpeechEvent::StartedSpeaking,
        );
        assert!(started.should_process());
        assert!(started.is_speech_start());
        assert!(!started.is_speech_end());

        let speaking = SpeechChunk::new(
            samples
                .clone()
                .into_iter()
                .map(|x| (x * 32767.0) as i16)
                .collect(),
            timestamp,
            SpeechEvent::Speaking,
        );
        assert!(speaking.should_process());
        assert!(!speaking.is_speech_start());
        assert!(!speaking.is_speech_end());

        let stopped = SpeechChunk::new(
            samples.into_iter().map(|x| (x * 32767.0) as i16).collect(),
            timestamp,
            SpeechEvent::StoppedSpeaking,
        );
        assert!(!stopped.should_process());
        assert!(!stopped.is_speech_start());
        assert!(stopped.is_speech_end());
    }

    #[test]
    fn test_duration_calculation() {
        let samples = vec![0.0; 256]; // 256 samples
        let chunk = SpeechChunk::new(
            samples.into_iter().map(|x| (x * 32767.0) as i16).collect(),
            Instant::now(),
            SpeechEvent::Speaking,
        );

        // 256 samples at 16kHz = 16ms
        assert!((chunk.duration_ms() - 16.0).abs() < 0.1);
    }

    #[test]
    fn test_speech_event_equality() {
        assert_eq!(SpeechEvent::StartedSpeaking, SpeechEvent::StartedSpeaking);
        assert_ne!(SpeechEvent::StartedSpeaking, SpeechEvent::Speaking);
        assert_ne!(SpeechEvent::Speaking, SpeechEvent::StoppedSpeaking);
    }
}
