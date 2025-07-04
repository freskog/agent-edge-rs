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

/// Audio capture configuration
#[derive(Debug, Clone)]
pub struct AudioCaptureConfig {
    pub device_id: Option<String>,
    pub channel: u32,
    pub sample_rate: u32,
    pub channels: u16,
    pub buffer_size: usize,
}

impl Default for AudioCaptureConfig {
    fn default() -> Self {
        Self {
            device_id: None,
            channel: 0,
            sample_rate: 16000, // 16kHz for audio processing
            channels: 1,        // Mono
            buffer_size: 1280,  // 80ms at 16kHz
        }
    }
}

/// Audio sink configuration
#[derive(Debug, Clone)]
pub struct AudioSinkConfig {
    pub device_id: Option<String>,
    pub sample_rate: u32,
    pub channels: u16,
    pub buffer_size: usize,
}

impl Default for AudioSinkConfig {
    fn default() -> Self {
        Self {
            device_id: None,
            sample_rate: 16000,
            channels: 1,
            buffer_size: 1280,
        }
    }
}

/// Audio sink trait for output
pub trait AudioSink: Send + Sync {
    fn play(&self, samples: &[f32]) -> Result<(), String>;
    fn stop(&self) -> Result<(), String>;
    fn write(&self, _samples: &[u8]) -> Result<(), String> {
        Ok(())
    }
}

/// Stub implementation for now - will be replaced with gRPC client
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

/// Audio hub trait for streaming
pub trait AudioHub {
    fn subscribe(&self) -> tokio::sync::broadcast::Receiver<AudioChunk>;
    fn audio_subscriber_count(&self) -> usize;
}

/// Stub implementation for now - will be replaced with gRPC client
pub struct StubAudioHub;

impl StubAudioHub {
    pub fn new(_config: AudioCaptureConfig) -> Result<Self, String> {
        Ok(Self)
    }
}

impl AudioHub for StubAudioHub {
    fn subscribe(&self) -> tokio::sync::broadcast::Receiver<AudioChunk> {
        let (tx, rx) = tokio::sync::broadcast::channel(128);
        // TODO: Connect to gRPC stream from audio_api
        rx
    }

    fn audio_subscriber_count(&self) -> usize {
        0 // TODO: Get from gRPC service
    }
}
