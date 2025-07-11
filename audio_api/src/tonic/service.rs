use crate::audio_converter::AudioConverter;
use crate::audio_sink::{AudioError, AudioSink, CpalConfig, CpalSink};
use crate::audio_source::{AudioCaptureConfig, CHUNK_SIZE};
use futures::StreamExt;
use log::{debug, error, info, warn};
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
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
    match &chunk.samples {
        Some(audio::audio_chunk::Samples::FloatSamples(bytes)) => {
            // Convert f32 bytes to f32 samples
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
            // Convert i16 bytes to f32 samples
            if bytes.len() % 2 != 0 {
                return Err(Status::invalid_argument("Invalid i16 sample data length"));
            }
            let mut samples = Vec::with_capacity(bytes.len() / 2);
            for chunk in bytes.chunks(2) {
                let i16_sample = i16::from_le_bytes([chunk[0], chunk[1]]);
                let f32_sample = i16_sample as f32 / 32768.0;
                samples.push(f32_sample);
            }
            Ok(samples)
        }
        Some(audio::audio_chunk::Samples::Int32Samples(bytes)) => {
            // Convert i32 bytes to f32 samples
            if bytes.len() % 4 != 0 {
                return Err(Status::invalid_argument("Invalid i32 sample data length"));
            }
            let mut samples = Vec::with_capacity(bytes.len() / 4);
            for chunk in bytes.chunks(4) {
                let i32_sample = i32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                let f32_sample = i32_sample as f32 / 2147483648.0;
                samples.push(f32_sample);
            }
            Ok(samples)
        }
        Some(audio::audio_chunk::Samples::Float64Samples(bytes)) => {
            // Convert f64 bytes to f32 samples
            if bytes.len() % 8 != 0 {
                return Err(Status::invalid_argument("Invalid f64 sample data length"));
            }
            let mut samples = Vec::with_capacity(bytes.len() / 8);
            for chunk in bytes.chunks(8) {
                let f64_sample = f64::from_le_bytes([
                    chunk[0], chunk[1], chunk[2], chunk[3], chunk[4], chunk[5], chunk[6], chunk[7],
                ]);
                samples.push(f64_sample as f32);
            }
            Ok(samples)
        }
        Some(audio::audio_chunk::Samples::Int24Samples(_)) => {
            // TODO: Implement i24 conversion
            Err(Status::unimplemented("i24 sample format not yet supported"))
        }
        None => Err(Status::invalid_argument("No samples data in AudioChunk")),
    }
}

/// Subscriber information for audio capture
#[derive(Debug)]
struct AudioSubscriber {
    sender: mpsc::Sender<Result<AudioChunk, Status>>,
}

/// Audio capture service that manages multiple subscribers
/// This service doesn't hold the actual AudioCapture to avoid Send/Sync issues
pub struct AudioCaptureService {
    subscribers: Arc<RwLock<HashMap<String, AudioSubscriber>>>,
    audio_sender: Option<mpsc::Sender<[f32; CHUNK_SIZE]>>,
    capture_sample_rate: u32,
    target_sample_rate: u32,
}

impl AudioCaptureService {
    pub fn new(config: AudioCaptureConfig) -> Result<Self, AudioError> {
        // We'll create the actual audio capture in a separate thread to avoid Send/Sync issues
        let (audio_tx, audio_rx) = mpsc::channel(100);

        // Detect the actual device sample rate
        let actual_sample_rate = Self::detect_device_sample_rate(&config)?;
        info!(
            "🎤 Detected device sample rate: {}Hz (config requested: {}Hz)",
            actual_sample_rate, config.sample_rate
        );

        let service = Self {
            subscribers: Arc::new(RwLock::new(HashMap::new())),
            audio_sender: Some(audio_tx),
            capture_sample_rate: actual_sample_rate,
            target_sample_rate: 16000, // Always output at 16kHz
        };

        // Start the audio distribution task
        service.start_audio_distribution(audio_rx);

        // Try to start audio capture in a separate thread
        service.start_audio_capture(config.clone());

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
            let actual_sample_rate = self.capture_sample_rate;

            // Update config to use the actual detected sample rate
            config.sample_rate = actual_sample_rate;

            // Spawn a blocking task for audio capture since CPAL isn't async-friendly
            std::thread::spawn(move || {
                use crate::audio_source::AudioCapture;

                // We don't need a sync channel for this implementation

                // Try to create audio capture
                let _capture = match AudioCapture::new(config.clone(), sender_clone) {
                    Ok(capture) => {
                        info!("🎤 Audio capture initialized successfully");
                        Some(capture)
                    }
                    Err(e) => {
                        warn!("🎤 Audio capture initialization failed: {} - service will run without capture", e);
                        None
                    }
                };

                // Keep the thread alive to maintain the audio capture
                // In a real implementation, you'd want a proper shutdown mechanism
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            });
        }
    }

    fn start_audio_distribution(&self, mut capture_receiver: mpsc::Receiver<[f32; CHUNK_SIZE]>) {
        let subscribers = Arc::clone(&self.subscribers);
        let capture_rate = self.capture_sample_rate;
        let target_rate = self.target_sample_rate;

        tokio::spawn(async move {
            info!(
                "🎤 Audio distribution task started ({}Hz -> {}Hz)",
                capture_rate, target_rate
            );

            // Create resampler if needed
            let mut resampler = if capture_rate != target_rate {
                let ratio = target_rate as f64 / capture_rate as f64;
                info!("🎤 Creating resampler with ratio: {:.3}", ratio);

                let params = SincInterpolationParameters {
                    sinc_len: 256,
                    f_cutoff: 0.95,
                    interpolation: SincInterpolationType::Linear,
                    oversampling_factor: 256,
                    window: WindowFunction::BlackmanHarris2,
                };

                match SincFixedIn::<f32>::new(
                    ratio, 2.0, // max_resample_ratio_relative
                    params, CHUNK_SIZE, 1, // channels
                ) {
                    Ok(resampler) => Some(resampler),
                    Err(e) => {
                        error!("Failed to create resampler: {}", e);
                        None
                    }
                }
            } else {
                None
            };

            let mut sample_buffer = Vec::new();

            loop {
                // Get the next audio chunk
                let chunk = match capture_receiver.recv().await {
                    Some(samples) => samples,
                    None => {
                        info!("🎤 Audio capture stream ended");
                        break;
                    }
                };

                // Resample if needed
                let resampled_samples = if let Some(ref mut resampler) = resampler {
                    // Convert chunk to Vec for resampler
                    let input_samples = vec![chunk.to_vec()];

                    match resampler.process(&input_samples, None) {
                        Ok(output) => {
                            if !output.is_empty() && !output[0].is_empty() {
                                output[0].clone()
                            } else {
                                continue; // Skip empty output
                            }
                        }
                        Err(e) => {
                            warn!("Resampling error: {}", e);
                            continue;
                        }
                    }
                } else {
                    chunk.to_vec()
                };

                // Buffer samples until we have exactly CHUNK_SIZE
                sample_buffer.extend_from_slice(&resampled_samples);

                while sample_buffer.len() >= CHUNK_SIZE {
                    // Extract exactly CHUNK_SIZE samples
                    let output_chunk: [f32; CHUNK_SIZE] =
                        sample_buffer[0..CHUNK_SIZE].try_into().unwrap();
                    sample_buffer.drain(0..CHUNK_SIZE);

                    // Convert to gRPC AudioChunk format
                    let audio_chunk = AudioChunk {
                        samples: Some(audio::audio_chunk::Samples::FloatSamples(
                            output_chunk.iter().flat_map(|&f| f.to_le_bytes()).collect(),
                        )),
                        timestamp_ms: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .as_millis() as u64,
                        format: Some(AudioFormat {
                            sample_rate: target_rate,
                            channels: 1,
                            sample_format: audio::SampleFormat::F32 as i32,
                        }),
                    };

                    // Send to all subscribers
                    let mut subscribers_to_remove = Vec::new();
                    {
                        let subscribers_read = subscribers.read().await;
                        for (id, subscriber) in subscribers_read.iter() {
                            if let Err(_) = subscriber.sender.try_send(Ok(audio_chunk.clone())) {
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

// Make AudioCaptureService Send + Sync by not holding the AudioCapture directly
unsafe impl Send for AudioCaptureService {}
unsafe impl Sync for AudioCaptureService {}

/// Active audio stream information
struct ActiveStream {
    stream_id: String,
    abort_tx: mpsc::Sender<()>,
    task_handle: tokio::task::JoinHandle<Result<(), AudioError>>,
}

/// Main audio service implementation
pub struct AudioServiceImpl {
    audio_sink: Arc<dyn AudioSink>,
    target_format: AudioFormat,
    capture_service: Arc<AudioCaptureService>,
    active_streams: Arc<RwLock<HashMap<String, mpsc::Sender<()>>>>, // stream_id -> abort_sender
}

impl AudioServiceImpl {
    pub fn new(audio_sink: Arc<dyn AudioSink>) -> Result<Self, AudioError> {
        let target_format = audio_sink.get_format();
        info!(
            "🔊 Audio sink created with target format: {}Hz, {}ch, {}",
            target_format.sample_rate, target_format.channels, target_format.sample_format
        );

        // Create capture service with default config
        let capture_service = Arc::new(AudioCaptureService::new(AudioCaptureConfig::default())?);

        Ok(Self {
            audio_sink,
            target_format,
            capture_service,
            active_streams: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    pub fn new_with_config(config: CpalConfig) -> Result<Self, AudioError> {
        let audio_sink = Arc::new(CpalSink::new(config)?);
        let target_format = audio_sink.get_format();
        info!(
            "🔊 Audio sink created with target format: {}Hz, {}ch, {}",
            target_format.sample_rate, target_format.channels, target_format.sample_format
        );

        // Create capture service with default config
        let capture_service = Arc::new(AudioCaptureService::new(AudioCaptureConfig::default())?);

        Ok(Self {
            audio_sink,
            target_format,
            capture_service,
            active_streams: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    pub fn new_with_capture_config(
        config: CpalConfig,
        capture_config: AudioCaptureConfig,
    ) -> Result<Self, AudioError> {
        let audio_sink = Arc::new(CpalSink::new(config)?);
        let target_format = audio_sink.get_format();
        info!(
            "🔊 Audio sink created with target format: {}Hz, {}ch, {}",
            target_format.sample_rate, target_format.channels, target_format.sample_format
        );

        // Create capture service with provided config
        let capture_service = Arc::new(AudioCaptureService::new(capture_config)?);

        Ok(Self {
            audio_sink,
            target_format,
            capture_service,
            active_streams: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    /// Handle audio playback for a specific stream
    async fn handle_playback(
        &self,
        mut stream: tonic::Streaming<PlayAudioRequest>,
    ) -> Result<PlayResponse, Status> {
        let mut current_stream_id = None;
        let mut converter: Option<AudioConverter> = None;
        let mut chunks_played = 0;
        let (abort_tx, mut abort_rx) = mpsc::channel::<()>(1);

        info!("🔊 Starting playback stream processing");

        loop {
            tokio::select! {
                // Check for abort signal
                _ = abort_rx.recv() => {
                    info!("🛑 Received abort signal for stream: {:?}", current_stream_id);
                    // Clean up the stream from active streams
                    if let Some(ref stream_id) = current_stream_id {
                        let mut active_streams = self.active_streams.write().await;
                        active_streams.remove(stream_id);
                    }
                    return Err(Status::aborted("Stream aborted by user request"));
                }

                // Process stream data
                request_result = stream.next() => {
                    let request = match request_result {
                        Some(Ok(req)) => req,
                        Some(Err(e)) => return Err(e),
                        None => break, // Stream ended
                    };

                    debug!("🔊 Received request from stream");

                    match request.data {
                Some(play_audio_request::Data::Chunk(chunk)) => {
                    // Initialize stream if this is the first chunk
                    if current_stream_id.is_none() {
                        current_stream_id = Some(request.stream_id.clone());
                        info!("🔊 Initializing playback stream: {}", request.stream_id);

                        // Register the stream for abort functionality
                        {
                            let mut active_streams = self.active_streams.write().await;
                            active_streams.insert(request.stream_id.clone(), abort_tx.clone());
                        }
                        info!("🔊 Stream registered for abort functionality: {}", request.stream_id);

                        // Extract format from first chunk
                        let input_format = chunk.format.as_ref().ok_or_else(|| {
                            Status::invalid_argument("First chunk must include format metadata")
                        })?;

                        // Create converter using pre-configured target format
                        let conv = AudioConverter::new(input_format, &self.target_format).map_err(
                            |e| Status::invalid_argument(format!("Audio format error: {e}")),
                        )?;
                        converter = Some(conv);
                        info!("🔊 Playback stream initialized successfully");
                    }

                    // Extract f32 samples from the chunk
                    let f32_samples = extract_f32_samples(&chunk)?;

                    debug!(
                        "🔊 Processing chunk {} with {} samples",
                        chunks_played + 1,
                        f32_samples.len()
                    );

                    // Validate chunk size (allow variable, just buffer)
                    if f32_samples.is_empty() {
                        return Err(Status::invalid_argument("Empty audio chunk"));
                    }

                    // Check for backpressure before processing
                    if self.audio_sink.is_backpressure_active() {
                        warn!(
                            "🔊 Backpressure detected - buffer at {}%, slowing down processing",
                            self.audio_sink.get_buffer_percentage()
                        );
                        // Add a small delay to help with backpressure
                        tokio::time::sleep(tokio::time::Duration::from_millis(5)).await;
                    }

                    // Feed samples to converter and write all available output
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
                            // The converter now returns data in the correct format for the sink
                            // Handle backpressure transparently - retry a few times before giving up
                            let mut retries = 0;
                            const MAX_RETRIES: u32 = 3;

                            loop {
                                match self.audio_sink.write(&processed).await {
                                    Ok(_) => break,
                                    Err(AudioError::BufferFull) if retries < MAX_RETRIES => {
                                        retries += 1;
                                        log::debug!(
                                            "🔊 Buffer full, retry {}/{} - applying natural backpressure",
                                            retries, MAX_RETRIES
                                        );
                                        // Use a small delay to let the audio buffer drain
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
                            debug!(
                                "🔊 Played chunk {} for stream {}",
                                chunks_played,
                                current_stream_id.as_ref().unwrap()
                            );
                        } else {
                            info!("🔊 Converter returned empty output, buffering samples");
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
                            // The converter now returns data in the correct format for the sink
                            self.audio_sink.write(&processed).await.map_err(|e| {
                                error!("Playback error: {}", e);
                                Status::internal(format!("Playback error: {}", e))
                            })?;
                            chunks_played += 1;
                        }
                    }
                    info!(
                        "🔊 Stream {} ended normally",
                        current_stream_id.as_ref().unwrap_or(&"unknown".to_string())
                    );
                    break;
                }
                None => {
                    return Err(Status::invalid_argument("Missing data in PlayAudioRequest"));
                }
            }
                }
            }
        }

        // Signal that no more audio will be sent
        info!("🔊 Signaling end of stream...");
        self.audio_sink.signal_end_of_stream().await.map_err(|e| {
            error!("Failed to signal end of stream: {}", e);
            Status::internal(format!("Failed to signal end of stream: {}", e))
        })?;

        // Wait for all audio to finish playing before returning
        info!("🔊 Waiting for audio playback to complete...");
        self.audio_sink.wait_for_completion().await.map_err(|e| {
            error!("Failed to wait for audio completion: {}", e);
            Status::internal(format!("Failed to wait for audio completion: {}", e))
        })?;

        // Clean up the stream from active streams
        if let Some(ref stream_id) = current_stream_id {
            let mut active_streams = self.active_streams.write().await;
            active_streams.remove(stream_id);
            info!("🔊 Stream cleaned up from active streams: {}", stream_id);
        }

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
            chunks_played: 0, // TODO: Track chunks played
        }))
    }

    async fn abort_playback(
        &self,
        request: Request<AbortRequest>,
    ) -> Result<Response<AbortResponse>, Status> {
        let stream_id = request.into_inner().stream_id;
        info!("🛑 Aborting audio playback: {}", stream_id);

        // Check if the stream exists and send abort signal
        let abort_sent = {
            let active_streams = self.active_streams.read().await;
            if let Some(abort_sender) = active_streams.get(&stream_id) {
                match abort_sender.try_send(()) {
                    Ok(_) => {
                        info!("🛑 Abort signal sent to stream: {}", stream_id);
                        true
                    }
                    Err(_) => {
                        warn!("🛑 Failed to send abort signal to stream: {}", stream_id);
                        false
                    }
                }
            } else {
                warn!("🛑 Stream not found for abort: {}", stream_id);
                false
            }
        };

        // Also abort the audio sink to clear any buffered audio immediately
        if let Err(e) = self.audio_sink.abort().await {
            error!("🛑 Failed to abort audio sink: {}", e);
        }

        let (success, message) = if abort_sent {
            (true, format!("Stream {} aborted successfully", stream_id))
        } else {
            (
                false,
                format!("Stream {} not found or already completed", stream_id),
            )
        };

        Ok(Response::new(AbortResponse { success, message }))
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
