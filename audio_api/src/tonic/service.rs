use super::capture_service::AudioCaptureService;
use crate::audio_sink::{AudioError, AudioSink, CpalConfig, CpalSink};
use crate::audio_source::AudioCaptureConfig;
use crate::platform::AudioPlatform;
use crate::platform_converter::{create_playback_converter, PlatformConverter};
use futures::StreamExt;
use log::{debug, error, info};

use std::sync::Arc;
use tokio::sync::mpsc;
use tonic::transport::Server;
use tonic::{Request, Response, Status};
use uuid::Uuid;

use service_protos::audio_service_server::{AudioService, AudioServiceServer};
use service_protos::{
    play_audio_request, AbortRequest, AbortResponse, AudioChunk, AudioFormat, EndStreamRequest,
    EndStreamResponse, PlayAudioRequest, PlayResponse, SubscribeRequest,
};

// Use the shared protobuf definitions
use service_protos::audio;

/// Helper function to extract f32 samples from AudioChunk
fn extract_f32_samples(chunk: &AudioChunk) -> Result<Vec<f32>, Status> {
    // Convert bytes to f32 samples based on format (only i16 and f32 supported)
    match &chunk.samples {
        Some(audio::audio_chunk::Samples::FloatSamples(bytes)) => {
            if bytes.len() % 4 != 0 {
                return Err(Status::invalid_argument("Invalid f32 sample data length"));
            }
            let mut samples = Vec::with_capacity(bytes.len() / 4);
            for chunk in bytes.chunks(4) {
                let f32_sample = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                samples.push(f32_sample);
            }
            Ok(samples)
        }
        Some(audio::audio_chunk::Samples::Int16Samples(bytes)) => {
            if bytes.len() % 2 != 0 {
                return Err(Status::invalid_argument("Invalid i16 sample data length"));
            }
            let mut samples = Vec::with_capacity(bytes.len() / 2);
            for chunk in bytes.chunks(2) {
                let i16_sample = i16::from_le_bytes([chunk[0], chunk[1]]);
                // Convert i16 to f32: scale from [-32768, 32767] to [-1.0, 1.0]
                let f32_sample = i16_sample as f32 / 32768.0;
                samples.push(f32_sample);
            }
            Ok(samples)
        }
        None => Err(Status::invalid_argument("Missing audio sample data")),
    }
}

/// Main audio service implementation
pub struct AudioServiceImpl {
    audio_sink: Arc<dyn AudioSink>,
    target_format: AudioFormat,
    capture_service: Arc<AudioCaptureService>,
    platform: AudioPlatform,
}

impl AudioServiceImpl {
    /// Create a new AudioServiceImpl with a custom audio sink
    pub fn new(audio_sink: Arc<dyn AudioSink>) -> Result<Self, AudioError> {
        let target_format = audio_sink.get_format();
        info!(
            "🔊 Audio sink created with target format: {}Hz, {}ch, {}",
            target_format.sample_rate, target_format.channels, target_format.sample_format
        );

        // Use default platform (RaspberryPi) for now - this should be passed from main
        let capture_service = Arc::new(AudioCaptureService::new(
            AudioPlatform::RaspberryPi,
            AudioCaptureConfig::default(),
        )?);

        Ok(Self {
            audio_sink,
            target_format,
            capture_service,
            platform: AudioPlatform::RaspberryPi, // Initialize platform
        })
    }

    /// Create a new AudioServiceImpl with CPAL configuration
    pub fn with_cpal_config(config: CpalConfig) -> Result<Self, AudioError> {
        let audio_sink = Arc::new(CpalSink::new(config)?);
        Self::new(audio_sink)
    }

    /// Create a new AudioServiceImpl with custom audio and capture configurations
    pub fn with_configs(
        audio_config: CpalConfig,
        capture_config: AudioCaptureConfig,
    ) -> Result<Self, AudioError> {
        let audio_sink = Arc::new(CpalSink::new(audio_config)?);
        let target_format = audio_sink.get_format();
        info!(
            "🔊 Audio sink created with target format: {}Hz, {}ch, {}",
            target_format.sample_rate, target_format.channels, target_format.sample_format
        );

        // Use default platform (RaspberryPi) for now - this should be passed from main
        let capture_service = Arc::new(AudioCaptureService::new(
            AudioPlatform::RaspberryPi,
            capture_config,
        )?);

        Ok(Self {
            audio_sink,
            target_format,
            capture_service,
            platform: AudioPlatform::RaspberryPi, // Initialize platform
        })
    }

    /// Create a new AudioServiceImpl with platform-specific configurations
    pub fn with_platform_configs(
        platform: AudioPlatform,
        audio_config: CpalConfig,
        capture_config: AudioCaptureConfig,
    ) -> Result<Self, AudioError> {
        let audio_sink = Arc::new(CpalSink::new(audio_config)?);
        let target_format = audio_sink.get_format();
        info!(
            "🔊 Audio sink created with target format: {}Hz, {}ch, {} (platform: {})",
            target_format.sample_rate,
            target_format.channels,
            target_format.sample_format,
            platform
        );

        let capture_service = Arc::new(AudioCaptureService::new(platform, capture_config)?);

        Ok(Self {
            audio_sink,
            target_format,
            capture_service,
            platform: platform, // Set the platform
        })
    }

    /// Handle audio playback for a specific stream
    async fn handle_playback(
        &self,
        mut stream: tonic::Streaming<PlayAudioRequest>,
    ) -> Result<PlayResponse, Status> {
        let mut converter: Option<PlatformConverter> = None;
        let mut chunks_played = 0;

        info!("🔊 Starting playback stream processing");

        while let Some(request_result) = stream.next().await {
            let request = request_result?;
            debug!("🔊 Received request from stream");

            match request.data {
                Some(play_audio_request::Data::Chunk(chunk)) => {
                    // Initialize converter if this is the first chunk
                    if converter.is_none() {
                        info!("🔊 Initializing playback stream: {}", request.stream_id);

                        let input_format = chunk.format.as_ref().ok_or_else(|| {
                            Status::invalid_argument("First chunk must include format metadata")
                        })?;

                        // Create platform-specific converter for playback
                        // Input: whatever format the client sends (TTS data)
                        // Output: platform's playback format (DAC)
                        let platform_playback = self.platform.playback_config();
                        let tts_format = self.platform.tts_format();

                        // For simplicity, assume TTS input matches our standard TTS format
                        let conv = PlatformConverter::new(
                            self.platform,
                            tts_format.sample_rate,
                            platform_playback.sample_rate,
                            tts_format.format,
                            platform_playback.format,
                        )
                        .map_err(|e| {
                            Status::invalid_argument(format!("Platform converter error: {e}"))
                        })?;

                        converter = Some(conv);
                        info!(
                            "🔊 Playback stream initialized successfully for platform: {}",
                            self.platform
                        );
                    }

                    // Extract f32 samples from the chunk
                    let f32_samples = extract_f32_samples(&chunk)?;

                    debug!(
                        "🔊 Processing chunk {} with {} samples",
                        chunks_played + 1,
                        f32_samples.len()
                    );

                    if f32_samples.is_empty() {
                        return Err(Status::invalid_argument("Empty audio chunk"));
                    }

                    // Process audio through converter
                    if let Some(ref mut conv) = converter.as_mut() {
                        let processed = conv.convert(&f32_samples).map_err(|e| {
                            Status::internal(format!("Audio conversion error: {e}"))
                        })?;

                        debug!(
                            "🔊 Converter processed {} samples into {} bytes",
                            f32_samples.len(),
                            processed.len()
                        );

                        if !processed.is_empty() {
                            // Apply mono->stereo conversion if needed (TTS is mono, hardware expects stereo)
                            let platform_config = self.platform.playback_config();
                            let final_bytes = if platform_config.channels == 2 {
                                // Convert mono samples to stereo samples, then back to bytes
                                use crate::platform_converter::mono_to_stereo;

                                // Convert bytes back to samples for stereo conversion
                                let mono_samples: Vec<f32> = match platform_config.format {
                                    crate::platform::PlatformSampleFormat::F32 => processed
                                        .chunks_exact(4)
                                        .map(|chunk| {
                                            f32::from_le_bytes([
                                                chunk[0], chunk[1], chunk[2], chunk[3],
                                            ])
                                        })
                                        .collect(),
                                    crate::platform::PlatformSampleFormat::I16 => processed
                                        .chunks_exact(2)
                                        .map(|chunk| {
                                            let i16_sample =
                                                i16::from_le_bytes([chunk[0], chunk[1]]);
                                            i16_sample as f32 / 32768.0
                                        })
                                        .collect(),
                                };

                                let stereo_samples = mono_to_stereo(&mono_samples);

                                // Convert stereo samples back to bytes
                                match platform_config.format {
                                    crate::platform::PlatformSampleFormat::F32 => {
                                        let mut bytes =
                                            Vec::with_capacity(stereo_samples.len() * 4);
                                        for &sample in &stereo_samples {
                                            bytes.extend_from_slice(&sample.to_le_bytes());
                                        }
                                        bytes
                                    }
                                    crate::platform::PlatformSampleFormat::I16 => {
                                        let mut bytes =
                                            Vec::with_capacity(stereo_samples.len() * 2);
                                        for &sample in &stereo_samples {
                                            let clamped = sample.clamp(-1.0, 1.0);
                                            let i16_sample = (clamped * 32767.0) as i16;
                                            bytes.extend_from_slice(&i16_sample.to_le_bytes());
                                        }
                                        bytes
                                    }
                                }
                            } else {
                                processed // Mono output, use as-is
                            };

                            // Write to audio sink with simple retry logic
                            let mut retries = 0;
                            const MAX_RETRIES: u32 = 3;

                            loop {
                                match self.audio_sink.write(&final_bytes).await {
                                    Ok(_) => break,
                                    Err(AudioError::BufferFull) if retries < MAX_RETRIES => {
                                        retries += 1;
                                        log::debug!(
                                            "🔊 Buffer full, retry {}/{}",
                                            retries,
                                            MAX_RETRIES
                                        );
                                        tokio::time::sleep(tokio::time::Duration::from_millis(10))
                                            .await;
                                        continue;
                                    }
                                    Err(e) => {
                                        error!("Failed to write audio data: {}", e);
                                        return Err(Status::internal(format!(
                                            "Playback error: {}",
                                            e
                                        )));
                                    }
                                }
                            }

                            chunks_played += 1;
                            debug!("🔊 Played chunk {}", chunks_played);
                        }
                    }
                }
                Some(play_audio_request::Data::EndStream(_)) => {
                    // Flush any remaining samples
                    if let Some(ref mut conv) = converter.as_mut() {
                        let processed = conv.flush().map_err(|e| {
                            Status::internal(format!("Audio conversion error: {e}"))
                        })?;

                        debug!("🔊 Flush processed {} bytes", processed.len());

                        if !processed.is_empty() {
                            // Apply same mono->stereo conversion as regular processing
                            let platform_config = self.platform.playback_config();
                            let final_bytes = if platform_config.channels == 2 {
                                use crate::platform_converter::mono_to_stereo;

                                let mono_samples: Vec<f32> = match platform_config.format {
                                    crate::platform::PlatformSampleFormat::F32 => processed
                                        .chunks_exact(4)
                                        .map(|chunk| {
                                            f32::from_le_bytes([
                                                chunk[0], chunk[1], chunk[2], chunk[3],
                                            ])
                                        })
                                        .collect(),
                                    crate::platform::PlatformSampleFormat::I16 => processed
                                        .chunks_exact(2)
                                        .map(|chunk| {
                                            let i16_sample =
                                                i16::from_le_bytes([chunk[0], chunk[1]]);
                                            i16_sample as f32 / 32768.0
                                        })
                                        .collect(),
                                };

                                let stereo_samples = mono_to_stereo(&mono_samples);

                                match platform_config.format {
                                    crate::platform::PlatformSampleFormat::F32 => {
                                        let mut bytes =
                                            Vec::with_capacity(stereo_samples.len() * 4);
                                        for &sample in &stereo_samples {
                                            bytes.extend_from_slice(&sample.to_le_bytes());
                                        }
                                        bytes
                                    }
                                    crate::platform::PlatformSampleFormat::I16 => {
                                        let mut bytes =
                                            Vec::with_capacity(stereo_samples.len() * 2);
                                        for &sample in &stereo_samples {
                                            let clamped = sample.clamp(-1.0, 1.0);
                                            let i16_sample = (clamped * 32767.0) as i16;
                                            bytes.extend_from_slice(&i16_sample.to_le_bytes());
                                        }
                                        bytes
                                    }
                                }
                            } else {
                                processed
                            };

                            self.audio_sink.write(&final_bytes).await.map_err(|e| {
                                error!("Playback error: {}", e);
                                Status::internal(format!("Playback error: {}", e))
                            })?;
                            chunks_played += 1;
                        }
                    }
                    info!("🔊 Stream ended normally");
                    break;
                }
                None => {
                    return Err(Status::invalid_argument("Missing data in PlayAudioRequest"));
                }
            }
        }

        // Signal end of stream and wait for completion
        info!("🔊 Signaling end of stream...");
        self.audio_sink.signal_end_of_stream().await.map_err(|e| {
            error!("Failed to signal end of stream: {}", e);
            Status::internal(format!("Failed to signal end of stream: {}", e))
        })?;

        info!("🔊 Waiting for audio playback to complete...");
        self.audio_sink.wait_for_completion().await.map_err(|e| {
            error!("Failed to wait for audio completion: {}", e);
            Status::internal(format!("Failed to wait for audio completion: {}", e))
        })?;

        info!("🔊 Playback completed: {} chunks played", chunks_played);
        Ok(PlayResponse {
            success: true,
            message: format!(
                "Playback completed successfully. {} chunks played.",
                chunks_played
            ),
        })
    }
}

#[tonic::async_trait]
impl AudioService for AudioServiceImpl {
    type SubscribeAudioStream =
        std::pin::Pin<Box<dyn futures::Stream<Item = Result<AudioChunk, Status>> + Send + 'static>>;

    async fn subscribe_audio(
        &self,
        _request: Request<SubscribeRequest>,
    ) -> Result<Response<Self::SubscribeAudioStream>, Status> {
        // Create a unique subscriber ID
        let subscriber_id = Uuid::new_v4().to_string();

        // Create a channel for this subscriber
        let (tx, rx) = mpsc::channel(100);

        // Add this subscriber to the capture service
        self.capture_service
            .add_subscriber(subscriber_id.clone(), tx)
            .await;

        info!("🎤 New audio subscriber: {}", subscriber_id);

        // Convert the receiver into a stream
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);

        // Create a stream that removes the subscriber when dropped
        let capture_service = Arc::clone(&self.capture_service);
        let stream_with_cleanup = async_stream::stream! {
            tokio::pin!(stream);
            while let Some(item) = stream.next().await {
                yield item;
            }
            // Clean up when stream ends
            capture_service.remove_subscriber(&subscriber_id).await;
            info!("🎤 Audio subscriber stream ended: {}", subscriber_id);
        };

        Ok(Response::new(Box::pin(stream_with_cleanup)))
    }

    async fn play_audio(
        &self,
        request: Request<tonic::Streaming<PlayAudioRequest>>,
    ) -> Result<Response<PlayResponse>, Status> {
        info!("🔊 Starting audio playback stream");
        let stream = request.into_inner();
        let result = self.handle_playback(stream).await;
        match &result {
            Ok(response) => info!("🔊 Audio playback completed: {}", response.message),
            Err(e) => error!("🔊 Audio playback failed: {}", e),
        }
        Ok(Response::new(result?))
    }

    async fn end_audio_stream(
        &self,
        request: Request<EndStreamRequest>,
    ) -> Result<Response<EndStreamResponse>, Status> {
        let stream_id = request.into_inner().stream_id;
        info!("⏹️ Ending audio stream: {}", stream_id);

        // Stream management removed - streams are handled per-request now
        Ok(Response::new(EndStreamResponse {
            success: true,
            message: "Stream ended successfully".into(),
            chunks_played: 0,
        }))
    }

    async fn abort_playback(
        &self,
        request: Request<AbortRequest>,
    ) -> Result<Response<AbortResponse>, Status> {
        let stream_id = request.into_inner().stream_id;
        info!("🛑 Aborting audio playback: {}", stream_id);

        // Simply abort the audio sink to clear any buffered audio
        match self.audio_sink.abort().await {
            Ok(_) => {
                info!(
                    "🛑 Audio sink aborted successfully for stream: {}",
                    stream_id
                );
                Ok(Response::new(AbortResponse {
                    success: true,
                    message: format!("Stream {} aborted successfully", stream_id),
                }))
            }
            Err(e) => {
                error!("🛑 Failed to abort audio sink: {}", e);
                Ok(Response::new(AbortResponse {
                    success: false,
                    message: format!("Failed to abort stream {}: {}", stream_id, e),
                }))
            }
        }
    }
}

/// Helper function to create and run the Tonic server on TCP
pub async fn run_server(
    addr: std::net::SocketAddr,
    service: AudioServiceImpl,
) -> Result<(), Box<dyn std::error::Error>> {
    let svc = AudioServiceServer::new(service);

    info!("🎵 Audio service listening on TCP {}", addr);

    Server::builder().add_service(svc).serve(addr).await?;

    Ok(())
}

/// Helper function to create and run the Tonic server on Unix domain socket
pub async fn run_server_unix(
    socket_path: &str,
    service: AudioServiceImpl,
) -> Result<(), Box<dyn std::error::Error>> {
    use std::path::Path;
    use tokio::net::UnixListener;
    use tokio_stream::wrappers::UnixListenerStream;

    let svc = AudioServiceServer::new(service);

    // Remove existing socket file if it exists
    if Path::new(socket_path).exists() {
        std::fs::remove_file(socket_path)?;
        info!("🗑️ Removed existing socket file: {}", socket_path);
    }

    // Create Unix domain socket listener
    let uds = UnixListener::bind(socket_path)?;
    info!(
        "🎵 Audio service listening on Unix domain socket: {}",
        socket_path
    );

    // Convert UnixListener to Tonic's transport using UnixListenerStream
    let incoming = UnixListenerStream::new(uds);

    Server::builder()
        .add_service(svc)
        .serve_with_incoming(incoming)
        .await?;

    Ok(())
}
