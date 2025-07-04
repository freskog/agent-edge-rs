pub mod events;

use crate::audio_capture::{AudioCapture, AudioCaptureConfig, AudioChunk};
use crate::error::EdgeError;
use crate::error::Result as EdgeResult;
use std::sync::Arc;
use tokio::sync::broadcast;

pub use events::SpeechChunk;

/// Broadcasts raw audio chunks ([f32; 1280]) to multiple subscribers.
pub struct SpeechHub {
    audio_tx: broadcast::Sender<SpeechChunk>,
    #[allow(dead_code)]
    audio_capture: Arc<AudioCapture>,
}

impl SpeechHub {
    pub fn new(audio_config: AudioCaptureConfig) -> EdgeResult<Self> {
        let (audio_tx, _) = broadcast::channel(128);
        let audio_tx_clone = audio_tx.clone();

        let audio_capture = AudioCapture::new(
            audio_config,
            Box::new(move |chunk: AudioChunk| {
                let mut samples_f32 = [0.0; 1280];
                for (i, &sample) in chunk.samples.iter().take(1280).enumerate() {
                    samples_f32[i] = sample as f32 / 32768.0;
                }
                let audio_chunk = SpeechChunk::new(
                    samples_f32,
                    std::time::Instant::now(),
                    crate::speech_producer::events::SpeechEvent::Speaking,
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

    pub fn subscribe_audio(&self) -> broadcast::Receiver<SpeechChunk> {
        self.audio_tx.subscribe()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<SpeechChunk> {
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
    fn test_speech_hub_creation() {
        // Skip audio device tests in CI/test environments
        if std::env::var("CI").is_ok() || std::env::var("GITHUB_ACTIONS").is_ok() {
            return;
        }

        let config = AudioCaptureConfig::default();
        let result = SpeechHub::new(config);

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
        let result = SpeechHub::new(config);

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
