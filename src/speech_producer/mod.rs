pub mod events;

use crate::audio_capture::{AudioCapture, AudioCaptureConfig, AudioChunk};
use crate::error::EdgeError;
use crate::error::Result as EdgeResult;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::broadcast;
use voice_activity_detector::VoiceActivityDetector;

pub use events::{SpeechChunk, SpeechEvent};

/// Speech hub that broadcasts speech events to multiple subscribers
pub struct SpeechHub {
    tx: broadcast::Sender<SpeechChunk>,
    #[allow(dead_code)]
    audio_capture: Arc<AudioCapture>, // Keep AudioCapture alive and accessible
}

impl SpeechHub {
    /// Create a new speech hub and start processing audio
    pub fn new(audio_config: AudioCaptureConfig, threshold: f32) -> EdgeResult<Self> {
        let (tx, _) = broadcast::channel(32); // Buffer up to 32 chunks
        let tx_clone = tx.clone();

        // Create VAD with our fixed settings
        let mut vad = VoiceActivityDetector::builder()
            .sample_rate(16000)
            .chunk_size(1280_usize)
            .build()
            .map_err(|err| EdgeError::VADError(err.to_string()))?;

        log::info!("🎤 Voice activity detector initialized (threshold: {:.2})", threshold);

        // Track speech state across callbacks
        let is_speaking = Arc::new(AtomicBool::new(false));
        let is_speaking_clone = is_speaking.clone();

        // Track trailing frames after speech ends
        let trailing_frames = Arc::new(AtomicUsize::new(0));
        let trailing_frames_clone = trailing_frames.clone();
        const TRAILING_FRAME_COUNT: usize = 5; // Send 5 extra frames after speech ends

        // Use different thresholds for different purposes
        let wakeword_threshold = threshold * 0.3; // More lenient for wakeword
        let speech_threshold = threshold * 0.5; // More lenient for speech events

        log::info!("🎤 Speech detection thresholds - wakeword: {:.2}, speech: {:.2}", 
                  wakeword_threshold, speech_threshold);

        // Create audio capture with callback that processes audio and broadcasts speech events
        let audio_capture = AudioCapture::new(
            audio_config,
            Box::new(move |chunk: AudioChunk| {
                // Check for speech using VAD with different thresholds
                let speech_prob = vad.predict(chunk.samples.clone());
                let was_speaking = is_speaking_clone.load(Ordering::Relaxed);
                let trailing_frame_count = trailing_frames_clone.load(Ordering::Relaxed);

                // Use different thresholds for different purposes
                let is_speech_for_events = speech_prob >= speech_threshold;
                let is_speech_for_wakeword = speech_prob >= wakeword_threshold;

                // Determine the appropriate event based on state transition
                let event = if is_speech_for_events {
                    // Reset trailing frames when we detect speech
                    trailing_frames_clone.store(0, Ordering::Relaxed);

                    if !was_speaking {
                        is_speaking_clone.store(true, Ordering::Relaxed);
                        log::debug!("🎤 Speech started (probability: {:.2})", speech_prob);
                        Some(SpeechEvent::StartedSpeaking)
                    } else {
                        log::debug!("🎤 Speech continuing (probability: {:.2})", speech_prob);
                        Some(SpeechEvent::Speaking)
                    }
                } else if was_speaking {
                    // No speech detected but we were speaking
                    if trailing_frame_count >= TRAILING_FRAME_COUNT {
                        // Only stop if we've sent enough trailing frames
                        is_speaking_clone.store(false, Ordering::Relaxed);
                        log::debug!("🎤 Speech stopped (after {} trailing frames)", trailing_frame_count);
                        Some(SpeechEvent::StoppedSpeaking)
                    } else {
                        // Still in trailing frame period, send as Speaking
                        trailing_frames_clone.fetch_add(1, Ordering::Relaxed);
                        log::debug!("🎤 Speech trailing frame {} of {}", trailing_frame_count + 1, TRAILING_FRAME_COUNT);
                        Some(SpeechEvent::Speaking)
                    }
                } else {
                    None
                };

                // Convert samples to f32 - we'll need this for all chunks
                let mut samples_f32 = [0.0; 1280];
                for (i, &sample) in chunk.samples.iter().take(1280).enumerate() {
                    samples_f32[i] = sample as f32 / 32768.0;
                }

                // Always send chunks for wakeword detection, regardless of speech activity
                let speech_chunk = SpeechChunk::new(
                    samples_f32,
                    std::time::Instant::now(),
                    event.unwrap_or(SpeechEvent::Speaking),
                );

                // Only log potential wakeword activity when speech starts
                if is_speech_for_wakeword && !was_speaking {
                    log::debug!("🎤 Potential wakeword activity detected (probability: {:.2})", speech_prob);
                }

                let _ = tx_clone.send(speech_chunk);
            }),
        )?;

        Ok(Self {
            tx,
            audio_capture: Arc::new(audio_capture),
        })
    }

    /// Subscribe to the speech stream
    pub fn subscribe(&self) -> broadcast::Receiver<SpeechChunk> {
        self.tx.subscribe()
    }

    /// Get the number of active subscribers
    pub fn subscriber_count(&self) -> usize {
        self.tx.receiver_count()
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
        let result = SpeechHub::new(config, 0.5);

        // In test environments without audio devices, this will fail
        // That's expected and okay
        match result {
            Ok(hub) => {
                assert_eq!(hub.subscriber_count(), 0);
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
        let result = SpeechHub::new(config, 0.5);

        match result {
            Ok(hub) => {
                let rx1 = hub.subscribe();
                let rx2 = hub.subscribe();

                assert_eq!(hub.subscriber_count(), 2);
                drop(rx1);
                assert_eq!(hub.subscriber_count(), 1);
                drop(rx2);
                assert_eq!(hub.subscriber_count(), 0);
            }
            Err(_) => {
                // Expected in test environments without audio devices
                println!("Audio device not available in test environment - this is expected");
            }
        }
    }
}
