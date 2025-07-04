//! Audio events and chunk definitions for the audio streamer

use std::time::Instant;

/// Audio events that can occur during audio processing
#[derive(Debug, Clone, PartialEq)]
pub enum AudioEvent {
    /// First audio chunk after silence - triggers processing
    StartedAudio,
    /// Ongoing audio chunk - continues processing
    Audio,
    /// First silence chunk after audio - signals end of audio
    StoppedAudio,
}

/// A chunk of audio with event information
#[derive(Debug, Clone)]
pub struct AudioChunk {
    /// Raw audio samples (1280 samples at 16kHz)
    pub samples: [f32; 1280],
    /// Timestamp when this chunk was captured
    pub timestamp: Instant,
    /// The audio event for this chunk
    pub audio_event: AudioEvent,
}

impl AudioChunk {
    /// Create a new audio chunk
    pub fn new(samples_f32: [f32; 1280], timestamp: Instant, audio_event: AudioEvent) -> Self {
        Self {
            samples: samples_f32,
            timestamp,
            audio_event,
        }
    }

    /// Returns true if this chunk contains audio (not silence)
    pub fn has_audio(&self) -> bool {
        matches!(
            self.audio_event,
            AudioEvent::StartedAudio | AudioEvent::Audio
        )
    }

    /// Returns true if this is the start of an audio segment
    pub fn is_audio_start(&self) -> bool {
        matches!(self.audio_event, AudioEvent::StartedAudio)
    }

    /// Returns true if this signals the end of an audio segment
    pub fn is_audio_end(&self) -> bool {
        matches!(self.audio_event, AudioEvent::StoppedAudio)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_chunk_creation() {
        let samples_f32 = [0.1; 1280];
        let timestamp = Instant::now();
        let chunk = AudioChunk::new(samples_f32, timestamp, AudioEvent::StartedAudio);

        assert_eq!(chunk.samples, [0.1; 1280]);
        assert_eq!(chunk.audio_event, AudioEvent::StartedAudio);
    }

    #[test]
    fn test_audio_chunk_events() {
        let samples = [0.1; 1280];
        let timestamp = Instant::now();

        let started = AudioChunk::new(samples, timestamp, AudioEvent::StartedAudio);
        assert!(started.has_audio());
        assert!(started.is_audio_start());
        assert!(!started.is_audio_end());

        let audio = AudioChunk::new(samples, timestamp, AudioEvent::Audio);
        assert!(audio.has_audio());
        assert!(!audio.is_audio_start());
        assert!(!audio.is_audio_end());

        let stopped = AudioChunk::new(samples, timestamp, AudioEvent::StoppedAudio);
        assert!(!stopped.has_audio());
        assert!(!stopped.is_audio_start());
        assert!(stopped.is_audio_end());
    }

    #[test]
    fn test_audio_chunk_timestamp() {
        let samples = [0.1; 1280];
        let timestamp = Instant::now();
        let chunk = AudioChunk::new(samples, timestamp, AudioEvent::Audio);

        assert_eq!(chunk.timestamp, timestamp);
    }
}
