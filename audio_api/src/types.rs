use serde::{Deserialize, Serialize};
use std::time::Instant;

pub const AUDIO_CHUNK_SIZE: usize = 1280;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WakewordConfig {
    pub model_path: String,
    pub threshold: f32,
    pub sensitivity: f32,
}

/// A fixed-size audio chunk with exactly 1280 samples
#[derive(Debug, Clone)]
pub struct AudioChunk {
    /// Raw audio samples (exactly 1280 samples at 16kHz)
    pub samples: [f32; AUDIO_CHUNK_SIZE],
    /// Timestamp when this chunk was captured
    pub timestamp: Instant,
}

impl AudioChunk {
    /// Create a new audio chunk
    pub fn new(samples: [f32; AUDIO_CHUNK_SIZE], timestamp: Instant) -> Self {
        Self { samples, timestamp }
    }

    /// Create from a vector, ensuring it's exactly the right size
    pub fn from_vec(samples: Vec<f32>) -> Result<Self, AudioChunkError> {
        let len = samples.len();
        if len != AUDIO_CHUNK_SIZE {
            return Err(AudioChunkError::InvalidSize {
                expected: AUDIO_CHUNK_SIZE,
                got: len,
            });
        }

        let samples_array: [f32; AUDIO_CHUNK_SIZE] =
            samples
                .try_into()
                .map_err(|_| AudioChunkError::InvalidSize {
                    expected: AUDIO_CHUNK_SIZE,
                    got: len,
                })?;

        Ok(Self {
            samples: samples_array,
            timestamp: Instant::now(),
        })
    }

    /// Get the samples as a slice
    pub fn samples(&self) -> &[f32] {
        &self.samples
    }

    /// Get the timestamp
    pub fn timestamp(&self) -> Instant {
        self.timestamp
    }
}

/// Errors that can occur when working with audio chunks
#[derive(Debug, thiserror::Error)]
pub enum AudioChunkError {
    #[error("Invalid chunk size: expected {expected}, got {got}")]
    InvalidSize { expected: usize, got: usize },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_chunk_creation() {
        let samples = [0.1; AUDIO_CHUNK_SIZE];
        let timestamp = Instant::now();
        let chunk = AudioChunk::new(samples, timestamp);

        assert_eq!(chunk.samples, [0.1; AUDIO_CHUNK_SIZE]);
        assert_eq!(chunk.timestamp, timestamp);
    }

    #[test]
    fn test_audio_chunk_timestamp() {
        let samples = [0.1; AUDIO_CHUNK_SIZE];
        let timestamp = Instant::now();
        let chunk = AudioChunk::new(samples, timestamp);

        assert_eq!(chunk.timestamp, timestamp);
    }

    #[test]
    fn test_audio_chunk_from_vec_valid() {
        let samples = vec![0.1; AUDIO_CHUNK_SIZE];
        let chunk = AudioChunk::from_vec(samples).unwrap();
        assert_eq!(chunk.samples.len(), AUDIO_CHUNK_SIZE);
    }

    #[test]
    fn test_audio_chunk_from_vec_invalid() {
        let samples = vec![0.1; 1000]; // Wrong size
        let result = AudioChunk::from_vec(samples);
        assert!(result.is_err());

        if let Err(AudioChunkError::InvalidSize { expected, got }) = result {
            assert_eq!(expected, AUDIO_CHUNK_SIZE);
            assert_eq!(got, 1000);
        } else {
            panic!("Expected InvalidSize error");
        }
    }
}
