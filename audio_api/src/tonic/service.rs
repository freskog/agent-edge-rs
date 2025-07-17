use super::capture_service::AudioCaptureService;
use crate::audio_sink::{AudioError, AudioSink, CpalConfig};
use crate::audio_source::AudioCaptureConfig;
use crate::platform::AudioPlatform;
use futures::StreamExt;
use log::{debug, error, info};

use std::sync::Arc;
use tokio::sync::mpsc;
use tonic::transport::Server;
use tonic::{Request, Response, Status};
use uuid::Uuid;

use service_protos::audio_service_server::{AudioService, AudioServiceServer};
use service_protos::{
    play_audio_request, AbortRequest, AbortResponse, AudioChunk, EndStreamRequest,
    EndStreamResponse, PlayAudioRequest, PlayResponse, SubscribeRequest,
};

/// Main audio service implementation
pub struct AudioServiceImpl {
    audio_sink: Arc<AudioSink>,
    capture_service: Arc<AudioCaptureService>,
}

impl AudioServiceImpl {
    /// Create a new AudioServiceImpl with a custom audio sink
    pub fn new(audio_sink: Arc<AudioSink>) -> Result<Self, AudioError> {
        info!("🔊 Audio sink created for streaming s16le playback");

        // Use default platform (RaspberryPi) for now - this should be passed from main
        let capture_service = Arc::new(AudioCaptureService::new(
            AudioPlatform::RaspberryPi,
            AudioCaptureConfig::default(),
        )?);

        Ok(Self {
            audio_sink,
            capture_service,
        })
    }

    /// Create a new AudioServiceImpl with CPAL configuration
    pub fn with_cpal_config(config: CpalConfig) -> Result<Self, AudioError> {
        let audio_sink = Arc::new(AudioSink::new(config)?);
        Self::new(audio_sink)
    }

    /// Create a new AudioServiceImpl with custom audio and capture configurations
    pub fn with_configs(
        audio_config: CpalConfig,
        capture_config: AudioCaptureConfig,
    ) -> Result<Self, AudioError> {
        let audio_sink = Arc::new(AudioSink::new(audio_config)?);
        info!("🔊 Audio sink created for streaming s16le playback");

        // Use default platform (RaspberryPi) for now - this should be passed from main
        let capture_service = Arc::new(AudioCaptureService::new(
            AudioPlatform::RaspberryPi,
            capture_config,
        )?);

        Ok(Self {
            audio_sink,
            capture_service,
        })
    }

    /// Create a new AudioServiceImpl with platform-specific configurations
    pub fn with_platform_configs(
        platform: AudioPlatform,
        audio_config: CpalConfig,
        capture_config: AudioCaptureConfig,
    ) -> Result<Self, AudioError> {
        let audio_sink = Arc::new(AudioSink::new(audio_config)?);
        info!(
            "🔊 Audio sink created for streaming s16le playback (platform: {})",
            platform
        );

        let capture_service = Arc::new(AudioCaptureService::new(platform, capture_config)?);

        Ok(Self {
            audio_sink,
            capture_service,
        })
    }

    /// Handle audio playback for a specific stream
    async fn handle_playback(
        &self,
        mut stream: tonic::Streaming<PlayAudioRequest>,
    ) -> Result<PlayResponse, Status> {
        let mut chunks_played = 0;

        info!("🔊 Starting streaming playback (s16le format, low latency)");

        // Abort any current playback to ensure new stream plays immediately
        if let Err(e) = self.audio_sink.abort().await {
            error!("Failed to abort current playback: {}", e);
            // Continue anyway - not critical
        }

        while let Some(request_result) = stream.next().await {
            let request = request_result?;
            debug!("🔊 Received request from stream");

            match request.data {
                Some(play_audio_request::Data::Chunk(chunk)) => {
                    // Stream s16le chunks immediately for low latency
                    debug!(
                        "🔊 Streaming chunk {} with {} bytes (s16le)",
                        chunks_played + 1,
                        chunk.samples.len()
                    );

                    if chunk.samples.is_empty() {
                        return Err(Status::invalid_argument("Empty audio chunk"));
                    }

                    // Write chunk immediately - returns right away for low latency
                    if let Err(e) = self.audio_sink.write_chunk(chunk.samples).await {
                        error!("Failed to write audio chunk: {}", e);
                        return Err(Status::internal(format!("Playback error: {}", e)));
                    }

                    chunks_played += 1;
                    debug!("🔊 Streamed chunk {} (low latency)", chunks_played);
                }
                Some(play_audio_request::Data::EndStream(_)) => {
                    info!("🔊 End of stream signal received");
                    break;
                }
                None => {
                    return Err(Status::invalid_argument("Missing data in PlayAudioRequest"));
                }
            }
        }

        // Now wait for true completion (user has heard the audio)
        info!("🔊 Waiting for streaming playback to complete...");
        if let Err(e) = self.audio_sink.end_stream_and_wait().await {
            error!("Failed to wait for audio completion: {}", e);
            return Err(Status::internal(format!("Completion error: {}", e)));
        }

        info!(
            "🔊 Streaming playback completed: {} chunks played",
            chunks_played
        );
        Ok(PlayResponse {
            success: true,
            message: format!(
                "Streaming playback completed successfully. {} chunks played.",
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
