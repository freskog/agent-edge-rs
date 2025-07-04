use serde::{Deserialize, Serialize};
use std::time::Instant;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WakewordConfig {
    pub model_path: String,
    pub threshold: f32,
    pub sensitivity: f32,
}

/// A chunk of audio data
#[derive(Debug, Clone)]
pub struct AudioChunk {
    /// Raw audio samples (1280 samples at 16kHz)
    pub samples: [f32; 1280],
    /// Timestamp when this chunk was captured
    pub timestamp: Instant,
}

impl AudioChunk {
    /// Create a new audio chunk
    pub fn new(samples_f32: [f32; 1280], timestamp: Instant) -> Self {
        Self {
            samples: samples_f32,
            timestamp,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_chunk_creation() {
        let samples_f32 = [0.1; 1280];
        let timestamp = Instant::now();
        let chunk = AudioChunk::new(samples_f32, timestamp);

        assert_eq!(chunk.samples, [0.1; 1280]);
        assert_eq!(chunk.timestamp, timestamp);
    }

    #[test]
    fn test_audio_chunk_timestamp() {
        let samples = [0.1; 1280];
        let timestamp = Instant::now();
        let chunk = AudioChunk::new(samples, timestamp);

        assert_eq!(chunk.timestamp, timestamp);
    }
}
