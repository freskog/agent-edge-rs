pub mod events;

use crate::audio_capture::{AudioCapture, AudioCaptureConfig, AudioChunk as RawAudioChunk};
use crate::error::EdgeError;
use crate::error::Result as EdgeResult;
use std::sync::Arc;
use tokio::sync::mpsc;

pub use events::AudioChunk;

/// Simple audio streamer for gRPC streaming
pub struct AudioStreamer {
    audio_capture: Arc<AudioCapture>,
}

impl AudioStreamer {
    pub fn new(audio_config: AudioCaptureConfig) -> EdgeResult<Self> {
        let (audio_tx, _audio_rx) = mpsc::channel(128);
        let audio_tx_clone = audio_tx.clone();

        let audio_capture = AudioCapture::new(
            audio_config,
            Box::new(move |chunk: RawAudioChunk| {
                let mut samples_f32 = [0.0; 1280];
                for (i, &sample) in chunk.samples.iter().take(1280).enumerate() {
                    samples_f32[i] = sample as f32 / 32768.0;
                }
                let audio_chunk = AudioChunk::new(
                    samples_f32,
                    std::time::Instant::now(),
                    crate::audio_streamer::events::AudioEvent::Audio,
                );
                let _ = audio_tx_clone.blocking_send(audio_chunk);
            }),
        )
        .map_err(EdgeError::from)?;

        Ok(Self {
            audio_capture: Arc::new(audio_capture),
        })
    }

    /// Get a receiver for audio chunks (for gRPC streaming)
    pub fn subscribe(&self) -> mpsc::Receiver<AudioChunk> {
        let (_tx, rx) = mpsc::channel(128);
        // TODO: Connect this to the audio capture when implementing gRPC
        rx
    }
}

// Keep the old AudioHub for backward compatibility during transition
pub struct AudioHub {
    audio_tx: tokio::sync::broadcast::Sender<AudioChunk>,
    #[allow(dead_code)]
    audio_capture: Arc<AudioCapture>,
}

impl AudioHub {
    pub fn new(audio_config: AudioCaptureConfig) -> EdgeResult<Self> {
        let (audio_tx, _) = tokio::sync::broadcast::channel(128);
        let audio_tx_clone = audio_tx.clone();

        let audio_capture = AudioCapture::new(
            audio_config,
            Box::new(move |chunk: RawAudioChunk| {
                let mut samples_f32 = [0.0; 1280];
                for (i, &sample) in chunk.samples.iter().take(1280).enumerate() {
                    samples_f32[i] = sample as f32 / 32768.0;
                }
                let audio_chunk = AudioChunk::new(
                    samples_f32,
                    std::time::Instant::now(),
                    crate::audio_streamer::events::AudioEvent::Audio,
                );
                let _ = audio_tx_clone.send(audio_chunk);
            }),
        )
        .map_err(EdgeError::from)?;

        Ok(Self {
            audio_tx,
            audio_capture: Arc::new(audio_capture),
        })
    }

    pub fn subscribe_audio(&self) -> tokio::sync::broadcast::Receiver<AudioChunk> {
        self.audio_tx.subscribe()
    }

    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<AudioChunk> {
        self.subscribe_audio()
    }

    pub fn audio_subscriber_count(&self) -> usize {
        self.audio_tx.receiver_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_hub_creation() {
        // Skip audio device tests in CI/test environments
        if std::env::var("CI").is_ok() || std::env::var("GITHUB_ACTIONS").is_ok() {
            return;
        }

        let config = AudioCaptureConfig::default();
        let result = AudioHub::new(config);

        // In test environments without audio devices, this will fail
        // That's expected and okay
        match result {
            Ok(hub) => {
                assert_eq!(hub.audio_subscriber_count(), 0);
            }
            Err(_) => {
                // Expected in test environments without audio devices
                println!("Audio device not available in test environment - this is expected");
            }
        }
    }

    #[test]
    fn test_subscription() {
        // Skip audio device tests in CI/test environments
        if std::env::var("CI").is_ok() || std::env::var("GITHUB_ACTIONS").is_ok() {
            return;
        }

        let config = AudioCaptureConfig::default();
        let result = AudioHub::new(config);

        match result {
            Ok(hub) => {
                let rx1 = hub.subscribe_audio();
                let rx2 = hub.subscribe_audio();

                assert_eq!(hub.audio_subscriber_count(), 2);
                drop(rx1);
                assert_eq!(hub.audio_subscriber_count(), 1);
                drop(rx2);
                assert_eq!(hub.audio_subscriber_count(), 0);
            }
            Err(_) => {
                // Expected in test environments without audio devices
                println!("Audio device not available in test environment - this is expected");
            }
        }
    }
}
