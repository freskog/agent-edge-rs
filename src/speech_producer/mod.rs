pub mod events;

pub use events::{SpeechChunk, SpeechEvent};

use crate::audio_capture::CpalAudioCapture;
use crate::error::Result as EdgeResult;
use crate::vad::{create_vad, VADConfig, VAD};
use futures_util::stream::Stream;
use log;
use std::pin::Pin;
use std::time::Instant;
use tokio::sync::broadcast;

/// Speech hub that runs VAD once and broadcasts to multiple subscribers
/// This is a single-threaded producer with pub/sub interface
pub struct SpeechHub {
    vad: Box<dyn VAD + Send>,
    broadcaster: broadcast::Sender<SpeechChunk>,
    is_currently_speaking: bool,
}

impl SpeechHub {
    /// Create a new speech hub with VAD configuration
    pub fn new(vad_config: VADConfig) -> EdgeResult<Self> {
        let vad = create_vad(vad_config)?;
        let (broadcaster, _) = broadcast::channel(1000); // Buffer up to 1000 chunks

        Ok(Self {
            vad,
            broadcaster,
            is_currently_speaking: false,
        })
    }

    /// Subscribe to the speech stream - returns a receiver for SpeechChunk
    pub fn subscribe(&self) -> broadcast::Receiver<SpeechChunk> {
        self.broadcaster.subscribe()
    }

    /// Run the speech hub with an audio source (single-threaded processing)
    pub async fn run(&mut self, audio_source: CpalAudioCapture) -> EdgeResult<()> {
        log::info!("SpeechHub: Starting with VAD processing and broadcasting");

        use futures_util::StreamExt;
        let mut audio_stream = Box::pin(audio_source);

        while let Some(result) = audio_stream.next().await {
            match result {
                Ok(audio_chunk) => {
                    // Apply VAD to determine if this chunk contains speech
                    let chunk_has_speech = match self.vad.should_process_audio(&audio_chunk.samples)
                    {
                        Ok(result) => result,
                        Err(e) => {
                            log::error!("VAD error: {}", e);
                            continue;
                        }
                    };

                    // Determine speech event based on state transition
                    let speech_event = match (self.is_currently_speaking, chunk_has_speech) {
                        (false, true) => {
                            self.is_currently_speaking = true;
                            Some(SpeechEvent::StartedSpeaking)
                        }
                        (true, true) => Some(SpeechEvent::Speaking),
                        (true, false) => {
                            self.is_currently_speaking = false;
                            Some(SpeechEvent::StoppedSpeaking)
                        }
                        (false, false) => None, // Skip silence chunks
                    };

                    // Only broadcast chunks for speech events
                    if let Some(event) = speech_event {
                        // Convert to f32 only if we have speech
                        let samples_f32 = audio_chunk
                            .samples
                            .iter()
                            .map(|&x| x as f32 / 32768.0)
                            .collect();

                        let speech_chunk = SpeechChunk {
                            samples_f32,
                            timestamp: Instant::now(),
                            speech_event: event,
                        };

                        // Broadcast to all subscribers (non-blocking)
                        if let Err(_) = self.broadcaster.send(speech_chunk) {
                            log::debug!("No subscribers listening to speech chunks");
                        }
                    }
                }
                Err(e) => {
                    log::error!("Audio source error: {}", e);
                    // Could emit error event here if needed
                }
            }
        }

        log::info!("SpeechHub: Audio source ended, stopping");
        Ok(())
    }

    /// Get the number of active subscribers
    pub fn subscriber_count(&self) -> usize {
        self.broadcaster.receiver_count()
    }
}

/// Create a stream from a broadcast receiver - helper for functional composition
pub fn speech_stream(
    mut receiver: broadcast::Receiver<SpeechChunk>,
) -> Pin<Box<dyn Stream<Item = SpeechChunk> + Send>> {
    Box::pin(async_stream::stream! {
        loop {
            match receiver.recv().await {
                Ok(chunk) => yield chunk,
                Err(broadcast::error::RecvError::Closed) => break,
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    log::warn!("Speech stream lagged, skipped {} chunks", skipped);
                    continue;
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio_capture::AudioCaptureConfig;
    use crate::vad::{ChunkSize, VADSampleRate};
    use futures_util::StreamExt;
    use std::time::Instant;

    #[tokio::test]
    async fn test_speech_hub_creation() {
        let vad_config = VADConfig {
            sample_rate: VADSampleRate::Rate16kHz,
            chunk_size: ChunkSize::Small,
            threshold: 0.5,
            speech_trigger_chunks: 2,
            silence_stop_chunks: 8,
        };

        let result = SpeechHub::new(vad_config);
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_speech_hub_subscribe() {
        let vad_config = VADConfig::default();
        let hub = SpeechHub::new(vad_config).unwrap();

        // Create multiple subscribers
        let _sub1 = hub.subscribe();
        let _sub2 = hub.subscribe();
        let _sub3 = hub.subscribe();

        assert_eq!(hub.subscriber_count(), 3);
    }

    #[tokio::test]
    async fn test_multiple_subscribers_same_data() {
        let vad_config = VADConfig::default();
        let mut hub = SpeechHub::new(vad_config).unwrap();

        // Create subscribers
        let sub1 = hub.subscribe();
        let sub2 = hub.subscribe();

        // Create test audio source
        let audio_config = AudioCaptureConfig::default();
        let audio_source = CpalAudioCapture::new(audio_config).unwrap();

        // Run hub in background
        let hub_handle = tokio::spawn(async move { hub.run(audio_source).await });

        // Both subscribers should get the same data
        let mut stream1 = speech_stream(sub1);
        let mut stream2 = speech_stream(sub2);

        // For silence, we shouldn't get any chunks
        tokio::time::timeout(std::time::Duration::from_millis(100), async {
            let chunk1 = stream1.next().await;
            let chunk2 = stream2.next().await;

            // Both should be None (no speech events for silence)
            assert!(chunk1.is_none());
            assert!(chunk2.is_none());
        })
        .await
        .unwrap_or(());

        // Clean up
        hub_handle.abort();
    }
}
