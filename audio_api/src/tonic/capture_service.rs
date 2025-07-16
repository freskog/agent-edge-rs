use crate::audio_source::{AudioCaptureConfig, CHUNK_SIZE};
use crate::platform::AudioPlatform;
use crate::platform_converter::{create_capture_converter, PlatformConverter};
use log::{debug, error, info, warn};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tonic::Status;

use crate::audio_sink::AudioError;
use service_protos::audio;
use service_protos::{AudioChunk, AudioFormat};

/// Subscriber information for audio capture
#[derive(Debug)]
pub struct AudioSubscriber {
    pub sender: mpsc::Sender<Result<AudioChunk, Status>>,
}

/// Audio capture service that manages multiple subscribers
pub struct AudioCaptureService {
    subscribers: Arc<RwLock<HashMap<String, AudioSubscriber>>>,
    audio_sender: Option<mpsc::Sender<[f32; CHUNK_SIZE]>>,
    platform: AudioPlatform,
}

impl AudioCaptureService {
    pub fn new(platform: AudioPlatform, config: AudioCaptureConfig) -> Result<Self, AudioError> {
        // We'll create the actual audio capture in a separate thread to avoid Send/Sync issues
        let (audio_tx, audio_rx) = mpsc::channel(100);

        // Get platform configuration
        let platform_config = platform.capture_config();
        info!(
            "🎤 Platform capture setup: {} ({})",
            platform_config.description, platform_config.preferred_format
        );

        let service = Self {
            subscribers: Arc::new(RwLock::new(HashMap::new())),
            audio_sender: Some(audio_tx),
            platform,
        };

        // Start the audio distribution task
        service.start_audio_distribution(audio_rx);

        // Start audio capture in a separate thread
        service.start_audio_capture(config);

        Ok(service)
    }

    fn detect_device_sample_rate(config: &AudioCaptureConfig) -> Result<u32, AudioError> {
        use cpal::traits::{DeviceTrait, HostTrait};

        let host = cpal::default_host();
        let device = if let Some(id) = &config.device_id {
            host.devices()
                .map_err(|e| AudioError::DeviceError(e.to_string()))?
                .find(|d| d.name().map(|n| n == *id).unwrap_or(false))
                .ok_or_else(|| AudioError::DeviceError(format!("Device not found: {}", id)))?
        } else {
            host.default_input_device()
                .ok_or_else(|| AudioError::DeviceError("No default input device found".into()))?
        };

        // First, try to find if the requested sample rate is supported
        let requested_rate = config.sample_rate;
        let supported_configs = device
            .supported_input_configs()
            .map_err(|e| AudioError::DeviceError(e.to_string()))?;

        for supported_config in supported_configs {
            let min_rate = supported_config.min_sample_rate().0;
            let max_rate = supported_config.max_sample_rate().0;

            if requested_rate >= min_rate && requested_rate <= max_rate {
                info!(
                    "🎤 Device supports requested sample rate: {}Hz",
                    requested_rate
                );
                return Ok(requested_rate);
            }
        }

        // If requested rate is not supported, fall back to default
        let default_config = device
            .default_input_config()
            .map_err(|e| AudioError::DeviceError(e.to_string()))?;

        let default_rate = default_config.sample_rate().0;
        info!(
            "🎤 Requested rate {}Hz not supported, using device default: {}Hz",
            requested_rate, default_rate
        );

        Ok(default_rate)
    }

    fn start_audio_capture(&self, mut config: AudioCaptureConfig) {
        if let Some(sender) = &self.audio_sender {
            let sender_clone = sender.clone();

            // Detect actual device sample rate
            let actual_sample_rate = match Self::detect_device_sample_rate(&config) {
                Ok(rate) => rate,
                Err(e) => {
                    error!("Failed to detect device sample rate: {}", e);
                    return;
                }
            };

            // Update config to use the actual detected sample rate
            config.sample_rate = actual_sample_rate;

            // Spawn a tokio task to handle audio capture with the new async interface
            tokio::spawn(async move {
                use crate::audio_source::AudioCapture;
                use futures::StreamExt;

                // Create audio capture with new async interface
                let capture = match AudioCapture::new(config).await {
                    Ok(capture) => {
                        info!("🎤 Audio capture initialized successfully");
                        capture
                    }
                    Err(e) => {
                        warn!("🎤 Audio capture initialization failed: {} - service will run without capture", e);
                        return;
                    }
                };

                // Process audio chunks using StreamExt combinators
                capture
                    .for_each(|chunk| {
                        let sender = sender_clone.clone();
                        async move {
                            if sender.send(chunk).await.is_err() {
                                info!("🎤 Audio capture receiver dropped, stopping capture");
                            }
                        }
                    })
                    .await;

                info!("🎤 Audio capture task ended");
            });
        }
    }

    fn start_audio_distribution(&self, mut capture_receiver: mpsc::Receiver<[f32; CHUNK_SIZE]>) {
        let subscribers = Arc::clone(&self.subscribers);
        let platform = self.platform;

        tokio::spawn(async move {
            info!(
                "🎤 Audio distribution task started for platform: {}",
                platform
            );

            // Create platform converter for capture (device rate -> STT format)
            // For now, assume device runs at platform's preferred rate
            let platform_config = platform.capture_config();
            let mut converter =
                match create_capture_converter(platform, platform_config.preferred_sample_rate) {
                    Ok(conv) => conv,
                    Err(e) => {
                        error!("Failed to create platform converter: {}", e);
                        return;
                    }
                };

            // Target format for STT/Wakeword (always 16kHz i16 mono)
            let target_format = AudioFormat {
                sample_rate: 16000,
                channels: 1,
                sample_format: audio::SampleFormat::I16 as i32,
            };

            loop {
                // Get the next audio chunk
                let chunk = match capture_receiver.recv().await {
                    Some(samples) => samples,
                    None => {
                        info!("🎤 Audio capture stream ended");
                        break;
                    }
                };

                // Convert samples using PlatformConverter
                match converter.convert(&chunk) {
                    Ok(converted_bytes) => {
                        if !converted_bytes.is_empty() {
                            // Create AudioChunk from converted bytes (i16 format)
                            let audio_chunk = AudioChunk {
                                samples: Some(audio::audio_chunk::Samples::Int16Samples(
                                    converted_bytes,
                                )),
                                timestamp_ms: std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap()
                                    .as_millis()
                                    as u64,
                                format: Some(target_format.clone()),
                            };

                            // Send to all subscribers
                            let mut subscribers_to_remove = Vec::new();
                            {
                                let subscribers_read = subscribers.read().await;
                                for (id, subscriber) in subscribers_read.iter() {
                                    if let Err(_) =
                                        subscriber.sender.try_send(Ok(audio_chunk.clone()))
                                    {
                                        debug!(
                                            "🎤 Subscriber {} channel full or closed, marking for removal",
                                            id
                                        );
                                        subscribers_to_remove.push(id.clone());
                                    }
                                }
                            }

                            // Remove disconnected subscribers
                            if !subscribers_to_remove.is_empty() {
                                let mut subscribers_write = subscribers.write().await;
                                for id in subscribers_to_remove {
                                    subscribers_write.remove(&id);
                                    debug!("🎤 Removed disconnected subscriber: {}", id);
                                }
                            }

                            // Log subscriber count periodically
                            if rand::random::<u8>() < 10 {
                                // ~4% chance per chunk
                                let count = subscribers.read().await.len();
                                if count > 0 {
                                    debug!("🎤 Broadcasting to {} subscribers", count);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Audio conversion error: {}", e);
                        continue;
                    }
                }
            }

            info!("🎤 Audio distribution task ended");
        });
    }

    pub async fn add_subscriber(
        &self,
        id: String,
        sender: mpsc::Sender<Result<AudioChunk, Status>>,
    ) {
        let subscriber = AudioSubscriber { sender };
        self.subscribers
            .write()
            .await
            .insert(id.clone(), subscriber);
        info!("🎤 Added audio subscriber: {}", id);
    }

    pub async fn remove_subscriber(&self, id: &str) {
        if self.subscribers.write().await.remove(id).is_some() {
            info!("🎤 Removed audio subscriber: {}", id);
        }
    }

    pub async fn subscriber_count(&self) -> usize {
        self.subscribers.read().await.len()
    }
}
