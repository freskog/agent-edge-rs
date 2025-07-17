use crate::audio_source::{AudioCapture, AudioCaptureConfig};
use crate::platform::AudioPlatform;
use log::{debug, info, warn};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tokio::time::{Duration, Instant};
use tonic::Status;

use service_protos::AudioChunk;

pub struct AudioCaptureService {
    subscribers: Arc<RwLock<HashMap<String, mpsc::Sender<Result<AudioChunk, Status>>>>>,
    _capture_handle: tokio::task::JoinHandle<()>,
}

impl AudioCaptureService {
    pub fn new(
        platform: AudioPlatform,
        capture_config: AudioCaptureConfig,
    ) -> Result<Self, crate::audio_sink::AudioError> {
        let subscribers: Arc<RwLock<HashMap<String, mpsc::Sender<Result<AudioChunk, Status>>>>> =
            Arc::new(RwLock::new(HashMap::new()));
        let subscribers_clone = Arc::clone(&subscribers);

        info!(
            "🎤 Starting audio capture service for platform: {} (s16le output)",
            platform
        );

        // Spawn the audio capture task
        let capture_handle = tokio::spawn(async move {
            if let Err(e) =
                Self::run_capture_loop(subscribers_clone, platform, capture_config).await
            {
                warn!("Audio capture loop ended with error: {}", e);
            }
        });

        Ok(Self {
            subscribers,
            _capture_handle: capture_handle,
        })
    }

    async fn run_capture_loop(
        subscribers: Arc<RwLock<HashMap<String, mpsc::Sender<Result<AudioChunk, Status>>>>>,
        _platform: AudioPlatform,
        capture_config: AudioCaptureConfig,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        debug!("🎤 Initializing audio capture...");

        // Create audio capture - now emits s16le directly
        let mut audio_capture = AudioCapture::new(capture_config)
            .await
            .map_err(|e| format!("Failed to create audio capture: {}", e))?;

        info!("🎤 Audio capture initialized successfully (s16le output)");

        let mut sample_count = 0;
        let mut last_log_time = Instant::now();

        while let Some(s16le_chunk) = audio_capture.next_chunk().await {
            let chunk_sample_count = s16le_chunk.len() / 2; // s16le = 2 bytes per sample
            sample_count += chunk_sample_count;

            // Log stats every 5 seconds
            if last_log_time.elapsed() >= Duration::from_secs(5) {
                info!(
                    "🎤 Audio capture stats: {} samples captured (s16le)",
                    sample_count
                );
                sample_count = 0;
                last_log_time = Instant::now();
            }

            // Create audio chunk with s16le data (no conversion needed!)
            let audio_chunk = AudioChunk {
                samples: s16le_chunk,
                timestamp_ms: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
            };

            // Send to all subscribers
            let subscribers_map = subscribers.read().await;
            let mut to_remove = Vec::new();

            for (subscriber_id, sender) in subscribers_map.iter() {
                match sender.try_send(Ok(audio_chunk.clone())) {
                    Ok(_) => {}
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        debug!(
                            "🎤 Subscriber {} channel full, dropping chunk",
                            subscriber_id
                        );
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => {
                        debug!("🎤 Subscriber {} disconnected", subscriber_id);
                        to_remove.push(subscriber_id.clone());
                    }
                }
            }

            // Remove disconnected subscribers
            drop(subscribers_map);
            if !to_remove.is_empty() {
                let mut subscribers_map = subscribers.write().await;
                for id in to_remove {
                    subscribers_map.remove(&id);
                }
            }
        }

        info!("🎤 Audio capture loop ended");
        Ok(())
    }

    pub async fn add_subscriber(
        &self,
        subscriber_id: String,
        sender: mpsc::Sender<Result<AudioChunk, Status>>,
    ) {
        let mut subscribers = self.subscribers.write().await;
        subscribers.insert(subscriber_id, sender);
    }

    pub async fn remove_subscriber(&self, subscriber_id: &str) {
        let mut subscribers = self.subscribers.write().await;
        subscribers.remove(subscriber_id);
    }
}
