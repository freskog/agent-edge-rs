use crate::{error::OpenWakeWordError, Model};
use futures::StreamExt;
use log::{debug, error, info, warn};
use service_protos::{audio_service_client::AudioServiceClient, AudioChunk, SubscribeRequest};
use std::collections::HashMap;
use tonic::transport::{Channel, Uri};
use tonic::Request;

/// gRPC client for connecting to audio_api and performing wake word detection
pub struct WakewordGrpcClient {
    model: Model,
    client: AudioServiceClient<Channel>,
    detection_threshold: f32,
}

impl WakewordGrpcClient {
    /// Create a new gRPC client that connects to audio_api via Unix socket
    pub async fn new(
        socket_path: &str,
        model_names: Vec<String>,
        detection_threshold: f32,
    ) -> Result<Self, OpenWakeWordError> {
        info!("🔌 Connecting to audio_api at {}", socket_path);

        // Create Unix socket connection (similar to grpc_tests.rs)
        let channel = {
            let socket_path = socket_path.to_string();
            let connector = tower::service_fn(move |_: Uri| {
                let socket_path = socket_path.clone();
                async move {
                    let stream = tokio::net::UnixStream::connect(socket_path).await?;
                    Ok::<_, std::io::Error>(hyper_util::rt::tokio::TokioIo::new(stream))
                }
            });

            tonic::transport::Endpoint::try_from("http://[::]:50051")
                .map_err(|e| {
                    OpenWakeWordError::IoError(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        format!("Invalid endpoint: {}", e),
                    ))
                })?
                .connect_with_connector(connector)
                .await
                .map_err(|e| {
                    OpenWakeWordError::IoError(std::io::Error::new(
                        std::io::ErrorKind::ConnectionRefused,
                        format!("Failed to connect to audio_api: {}", e),
                    ))
                })?
        };

        let client = AudioServiceClient::new(channel);

        // Initialize the wakeword model
        info!(
            "🧠 Initializing wake word model with {} models",
            model_names.len()
        );
        let model = Model::new(
            model_names,
            vec![], // Empty metadata for now
            0.5,    // Default VAD threshold
            0.5,    // Default custom verifier threshold
        )?;

        info!(
            "✅ gRPC client initialized with detection threshold {}",
            detection_threshold
        );

        Ok(Self {
            model,
            client,
            detection_threshold,
        })
    }

    /// Start listening for audio and detecting wake words
    pub async fn start_detection(&mut self) -> Result<(), OpenWakeWordError> {
        info!("🎯 Starting wake word detection...");

        // Subscribe to audio stream
        let request = Request::new(SubscribeRequest {});
        let mut stream = self
            .client
            .subscribe_audio(request)
            .await
            .map_err(|e| {
                OpenWakeWordError::IoError(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("Failed to subscribe to audio: {}", e),
                ))
            })?
            .into_inner();

        info!("📡 Subscribed to audio stream, processing chunks...");

        let mut audio_buffer = Vec::new();
        let mut chunk_count = 0;
        let expected_sample_rate = 16000; // Wake word models expect 16kHz

        while let Some(chunk_result) = stream.next().await {
            match chunk_result {
                Ok(chunk) => {
                    chunk_count += 1;

                    if let Err(e) = self
                        .process_audio_chunk(&chunk, &mut audio_buffer, expected_sample_rate)
                        .await
                    {
                        warn!("Failed to process audio chunk {}: {}", chunk_count, e);
                        continue;
                    }

                    // Log progress periodically
                    if chunk_count % 100 == 0 {
                        debug!("Processed {} audio chunks", chunk_count);
                    }
                }
                Err(e) => {
                    error!("Error receiving audio chunk: {}", e);
                    // Continue processing, don't break the loop
                }
            }
        }

        warn!("Audio stream ended");
        Ok(())
    }

    /// Process a single audio chunk and perform detection
    async fn process_audio_chunk(
        &mut self,
        chunk: &AudioChunk,
        audio_buffer: &mut Vec<i16>,
        expected_sample_rate: u32,
    ) -> Result<(), OpenWakeWordError> {
        // Check format metadata (present in first chunk)
        if let Some(format) = &chunk.format {
            info!(
                "📊 Audio format: {}Hz, {} channels, {:?}",
                format.sample_rate, format.channels, format.sample_format
            );

            if format.sample_rate != expected_sample_rate {
                warn!(
                    "⚠️  Sample rate mismatch: got {}Hz, expected {}Hz",
                    format.sample_rate, expected_sample_rate
                );
            }

            if format.channels != 1 {
                warn!(
                    "⚠️  Multi-channel audio detected: {} channels (only mono supported)",
                    format.channels
                );
            }
        }

        // Extract audio samples based on format
        let samples = match &chunk.samples {
            Some(service_protos::audio_chunk::Samples::Int16Samples(bytes)) => {
                // Convert bytes to i16 samples (little-endian)
                bytes
                    .chunks_exact(2)
                    .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
                    .collect::<Vec<i16>>()
            }
            Some(service_protos::audio_chunk::Samples::FloatSamples(bytes)) => {
                // Convert bytes to f32, then to i16 (standard conversion, no scaling)
                let mut max_f32 = 0.0f32;
                let mut max_i16 = 0i16;
                let mut f32_samples = Vec::new();

                let samples: Vec<i16> = bytes
                    .chunks_exact(4)
                    .enumerate()
                    .map(|(i, chunk)| {
                        let float_sample =
                            f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                        max_f32 = max_f32.max(float_sample.abs());

                        // Store first few samples for detailed logging
                        if i < 10 {
                            f32_samples.push(float_sample);
                        }

                        // Standard F32 [-1,1] to i16 conversion
                        let i16_sample = (float_sample * i16::MAX as f32) as i16;
                        max_i16 = max_i16.max(i16_sample.abs());
                        i16_sample
                    })
                    .collect();

                // Detailed audio debugging
                debug!("🔍 Raw F32 samples (first 10): {:?}", f32_samples);
                debug!("🔍 F32 max_abs={:.8}, i16 max_abs={}", max_f32, max_i16);

                // Check if audio is essentially silent
                if max_f32 < 0.001 {
                    warn!(
                        "⚠️  Audio appears to be very quiet or silent! F32 max = {:.8}",
                        max_f32
                    );
                }

                // Log actual audio levels to understand the scale
                if max_f32 > 0.0 {
                    debug!(
                        "📊 Audio conversion: F32 max={:.6}, i16 max={}, ratio={:.1}",
                        max_f32,
                        max_i16,
                        max_i16 as f32 / max_f32
                    );
                }
                samples
            }
            Some(other) => {
                debug!("Unsupported sample format: {:?}", other);
                return Ok(());
            }
            None => {
                debug!("Empty audio chunk received");
                return Ok(());
            }
        };

        debug!("📦 Received {} samples", samples.len());

        // Show sample statistics for debugging
        if !samples.is_empty() {
            let max_sample = samples.iter().map(|&x| x.abs()).max().unwrap_or(0);
            let avg_sample =
                samples.iter().map(|&x| x.abs() as u32).sum::<u32>() / samples.len() as u32;
            debug!("📊 Sample levels: max={}, avg={}", max_sample, avg_sample);
        }

        // Add samples to buffer
        audio_buffer.extend_from_slice(&samples);
        debug!("📊 Buffer now has {} samples", audio_buffer.len());

        // Process audio when we have enough samples (e.g., 1 second = 16000 samples)
        const DETECTION_WINDOW_SAMPLES: usize = 16000; // 1 second at 16kHz

        if audio_buffer.len() >= DETECTION_WINDOW_SAMPLES {
            // Take the most recent window for detection
            let detection_samples = audio_buffer
                .iter()
                .skip(audio_buffer.len().saturating_sub(DETECTION_WINDOW_SAMPLES))
                .copied()
                .collect::<Vec<i16>>();

            info!(
                "🔍 Running wake word detection on {} samples",
                detection_samples.len()
            );

            // Perform wake word detection
            match self.model.predict(&detection_samples, None, 1.0) {
                Ok(predictions) => {
                    info!("📊 Detection results: {:?}", predictions);
                    self.handle_predictions(predictions).await;
                }
                Err(e) => {
                    warn!("Wake word detection failed: {}", e);
                }
            }

            // Keep buffer from growing too large - keep last 2 seconds
            const MAX_BUFFER_SAMPLES: usize = 32000; // 2 seconds at 16kHz
            if audio_buffer.len() > MAX_BUFFER_SAMPLES {
                let keep_from = audio_buffer.len() - MAX_BUFFER_SAMPLES;
                audio_buffer.drain(0..keep_from);
            }
        }

        Ok(())
    }

    /// Handle wake word detection results
    async fn handle_predictions(&self, predictions: HashMap<String, f32>) {
        for (model_name, confidence) in predictions {
            if confidence > self.detection_threshold {
                info!(
                    "🎯 WAKE WORD DETECTED: '{}' with confidence {:.3}",
                    model_name, confidence
                );

                // TODO: Add metrics, webhooks, or other actions here
            } else {
                // Log all confidence scores at info level for debugging
                info!(
                    "🔍 Detection result: '{}' with confidence {:.3} (threshold: {:.3})",
                    model_name, confidence, self.detection_threshold
                );
            }
        }
    }
}

/// Convenience function to create and start a wake word detection client
pub async fn start_wakeword_detection(
    socket_path: &str,
    model_names: Vec<String>,
    detection_threshold: f32,
) -> Result<(), OpenWakeWordError> {
    let mut client = WakewordGrpcClient::new(socket_path, model_names, detection_threshold).await?;
    client.start_detection().await
}
