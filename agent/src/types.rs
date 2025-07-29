use std::collections::VecDeque;
use std::time::Instant;

/// Represents different types of audio events for speech processing
#[derive(Debug, Clone, PartialEq)]
pub enum AudioEvent {
    StartedAudio, // User started speaking
    Audio,        // User is speaking
    StoppedAudio, // User stopped speaking (End of Speech)
}

/// Represents a chunk of audio data
#[derive(Debug, Clone)]
pub struct AudioChunk {
    pub data: Vec<f32>,
    pub sample_rate: u32,
    pub channels: u16,
    pub timestamp: Instant,
}

/// Configuration for audio processing
#[derive(Debug, Clone)]
pub struct AudioConfig {
    pub sample_rate: u32,
    pub channels: u16,
    pub buffer_size: usize,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            sample_rate: 16000,
            channels: 1,
            buffer_size: 1024,
        }
    }
}

/// Audio source trait (now blocking)
pub trait AudioSource: Send + Sync {
    /// Get the next audio chunk (blocking)
    fn next_chunk(&mut self) -> Option<AudioChunk>;

    /// Get the audio configuration
    fn config(&self) -> &AudioConfig;
}

/// Audio sink trait for output (needed by TTS)
pub trait AudioSink: Send + Sync {
    fn play(&self, samples: &[f32]) -> Result<(), String>;
    fn stop(&self) -> Result<(), String>;
    fn write(&self, _samples: &[u8]) -> Result<(), String> {
        Ok(())
    }
}

/// Audio sink configuration
#[derive(Debug, Clone)]
pub struct AudioSinkConfig {
    pub sample_rate: u32,
    pub channels: u16,
    pub buffer_size: usize,
}

impl Default for AudioSinkConfig {
    fn default() -> Self {
        Self {
            sample_rate: 16000,
            channels: 1,
            buffer_size: 1024,
        }
    }
}

/// Stub audio sink for testing
pub struct StubAudioSink;

impl StubAudioSink {
    pub fn new(_config: AudioSinkConfig) -> Result<Self, String> {
        Ok(Self)
    }
}

impl AudioSink for StubAudioSink {
    fn play(&self, _samples: &[f32]) -> Result<(), String> {
        Ok(())
    }
    fn stop(&self) -> Result<(), String> {
        Ok(())
    }
    fn write(&self, _samples: &[u8]) -> Result<(), String> {
        Ok(())
    }
}

/// Stub audio source for testing
pub struct StubAudioSource {
    config: AudioConfig,
    buffer: VecDeque<AudioChunk>,
}

impl StubAudioSource {
    pub fn new(config: AudioConfig) -> Self {
        Self {
            config,
            buffer: VecDeque::new(),
        }
    }

    pub fn add_chunk(&mut self, chunk: AudioChunk) {
        self.buffer.push_back(chunk);
    }
}

impl AudioSource for StubAudioSource {
    fn next_chunk(&mut self) -> Option<AudioChunk> {
        self.buffer.pop_front()
    }

    fn config(&self) -> &AudioConfig {
        &self.config
    }
}
