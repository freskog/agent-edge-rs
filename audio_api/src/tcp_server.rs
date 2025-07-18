use crate::audio_sink::{AudioSink, AudioSinkConfig};
use crate::audio_source::{AudioCapture, AudioCaptureConfig};
use crate::tcp_protocol::{Connection, Message, ProtocolError};
use std::collections::HashMap;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ServerError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Protocol error: {0}")]
    Protocol(#[from] ProtocolError),

    #[error("Audio error: {0}")]
    Audio(String),

    #[error("Too many connections (max: {max})")]
    TooManyConnections { max: usize },
}

/// Configuration for the TCP audio server
#[derive(Clone)]
pub struct ServerConfig {
    pub bind_address: String,
    pub max_connections: usize,
    pub audio_sink_config: AudioSinkConfig,
    pub audio_capture_config: AudioCaptureConfig,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind_address: "127.0.0.1:50051".to_string(),
            max_connections: 5,
            audio_sink_config: AudioSinkConfig::default(),
            audio_capture_config: AudioCaptureConfig::default(),
        }
    }
}

/// TCP audio server
pub struct AudioServer {
    config: ServerConfig,
    should_stop: Arc<AtomicBool>,
    connection_count: Arc<AtomicUsize>,
    audio_sink: Arc<Mutex<Option<AudioSink>>>,
    audio_capture: Arc<Mutex<Option<AudioCapture>>>,
    capture_subscribers: Arc<Mutex<HashMap<String, crossbeam::channel::Sender<Vec<u8>>>>>,
    forwarding_thread_running: Arc<AtomicBool>,
}

impl AudioServer {
    pub fn new(config: ServerConfig) -> Result<Self, ServerError> {
        Ok(Self {
            config,
            should_stop: Arc::new(AtomicBool::new(false)),
            connection_count: Arc::new(AtomicUsize::new(0)),
            audio_sink: Arc::new(Mutex::new(None)),
            audio_capture: Arc::new(Mutex::new(None)),
            capture_subscribers: Arc::new(Mutex::new(HashMap::new())),
            forwarding_thread_running: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Start the server (blocking)
    pub fn run(&self) -> Result<(), ServerError> {
        log::info!(
            "🎵 Starting TCP audio server on {}",
            self.config.bind_address
        );

        // Initialize audio components lazily when first needed
        let listener = TcpListener::bind(&self.config.bind_address)?;
        listener.set_nonblocking(true)?;

        log::info!("🎵 Server listening on {}", self.config.bind_address);

        // Basic signal handling
        let should_stop = Arc::clone(&self.should_stop);
        ctrlc::set_handler(move || {
            log::info!("🛑 Received shutdown signal");
            should_stop.store(true, Ordering::SeqCst);
        })
        .ok(); // Ignore error if ctrlc not available

        while !self.should_stop.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((stream, addr)) => {
                    let current_connections = self.connection_count.load(Ordering::SeqCst);
                    if current_connections >= self.config.max_connections {
                        log::warn!(
                            "⚠️  Rejecting connection from {}: too many connections ({}/{})",
                            addr,
                            current_connections,
                            self.config.max_connections
                        );
                        // Send error and close connection
                        if let Ok(mut conn) = Connection::new(stream) {
                            let error_msg = Message::ErrorResponse {
                                message: format!(
                                    "Too many connections (max: {})",
                                    self.config.max_connections
                                ),
                            };
                            let _ = conn.write_message(&error_msg);
                        }
                        continue;
                    }

                    log::info!(
                        "🔌 New connection from {} ({}/{})",
                        addr,
                        current_connections + 1,
                        self.config.max_connections
                    );

                    self.connection_count.fetch_add(1, Ordering::SeqCst);

                    // Spawn thread for this connection
                    let connection_count = Arc::clone(&self.connection_count);
                    let audio_sink = Arc::clone(&self.audio_sink);
                    let audio_capture = Arc::clone(&self.audio_capture);
                    let capture_subscribers = Arc::clone(&self.capture_subscribers);
                    let forwarding_thread_running = Arc::clone(&self.forwarding_thread_running);
                    let config = self.config.clone();

                    thread::spawn(move || {
                        if let Err(e) = Self::handle_connection(
                            stream,
                            config,
                            audio_sink,
                            audio_capture,
                            capture_subscribers,
                            forwarding_thread_running,
                        ) {
                            log::error!("Connection error: {}", e);
                        }

                        connection_count.fetch_sub(1, Ordering::SeqCst);
                        log::info!("🔌 Connection from {} closed", addr);
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    // No connections available, sleep briefly
                    thread::sleep(Duration::from_millis(10));
                }
                Err(e) => {
                    log::error!("Failed to accept connection: {}", e);
                    thread::sleep(Duration::from_millis(100));
                }
            }
        }

        log::info!("🛑 Server shutting down...");
        Ok(())
    }

    /// Stop the server
    pub fn stop(&self) {
        self.should_stop.store(true, Ordering::SeqCst);
    }

    /// Handle a single client connection
    fn handle_connection(
        stream: TcpStream,
        config: ServerConfig,
        audio_sink: Arc<Mutex<Option<AudioSink>>>,
        audio_capture: Arc<Mutex<Option<AudioCapture>>>,
        capture_subscribers: Arc<Mutex<HashMap<String, crossbeam::channel::Sender<Vec<u8>>>>>,
        forwarding_thread_running: Arc<AtomicBool>,
    ) -> Result<(), ServerError> {
        // Set stream to non-blocking for audio streaming
        stream.set_nonblocking(true)?;
        let mut conn = Connection::new(stream)?;
        let client_id = format!("client_{:?}", thread::current().id());
        let mut audio_rx: Option<crossbeam::channel::Receiver<Vec<u8>>> = None;

        loop {
            let mut did_work = false;

            // Use crossbeam::select! for efficient waiting when we have audio subscription
            if let Some(ref rx) = audio_rx {
                crossbeam::channel::select! {
                    recv(rx) -> data => {
                        if let Ok(audio_data) = data {
                            let chunk_msg = Message::AudioChunk {
                                audio_data,
                                timestamp_ms: std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_millis() as u64,
                            };

                            if conn.write_message(&chunk_msg).is_err() {
                                log::debug!(
                                    "🎤 Failed to send audio to {}, closing connection",
                                    client_id
                                );
                                break;
                            }
                            did_work = true;
                        }
                    }
                    default(Duration::from_millis(1)) => {
                        // Fall through to check for incoming messages
                    }
                }
            }

            match conn.read_message() {
                Ok(message) => {
                    did_work = true;
                    match Self::handle_message(
                        &mut conn,
                        message,
                        &client_id,
                        &config,
                        &audio_sink,
                        &audio_capture,
                        &capture_subscribers,
                        &forwarding_thread_running,
                        &mut audio_rx,
                    ) {
                        Ok(should_continue) => {
                            if !should_continue {
                                break;
                            }
                        }
                        Err(e) => {
                            log::error!("Error handling message: {}", e);
                            let error_msg = Message::ErrorResponse {
                                message: format!("Server error: {}", e),
                            };
                            let _ = conn.write_message(&error_msg);
                            break;
                        }
                    }
                }
                Err(ProtocolError::Io(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    // Client disconnected
                    break;
                }
                Err(ProtocolError::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    // No message available - this is expected in non-blocking mode
                }
                Err(e) => {
                    log::error!("Protocol error: {}", e);
                    let error_msg = Message::ErrorResponse {
                        message: format!("Protocol error: {}", e),
                    };
                    let _ = conn.write_message(&error_msg);
                    break;
                }
            }

            // Only sleep if we're truly idle (no audio, no messages)
            if !did_work {
                thread::sleep(Duration::from_millis(1));
            }
        }

        // Clean up audio capture subscription if any
        {
            let mut subscribers = capture_subscribers.lock().unwrap();
            subscribers.remove(&client_id);
        }
        log::debug!("🎤 Removed subscriber: {}", client_id);

        Ok(())
    }

    /// Handle a single message, returns Ok(should_continue)
    fn handle_message(
        conn: &mut Connection,
        message: Message,
        client_id: &str,
        config: &ServerConfig,
        audio_sink: &Arc<Mutex<Option<AudioSink>>>,
        audio_capture: &Arc<Mutex<Option<AudioCapture>>>,
        capture_subscribers: &Arc<Mutex<HashMap<String, crossbeam::channel::Sender<Vec<u8>>>>>,
        forwarding_thread_running: &Arc<AtomicBool>,
        audio_rx: &mut Option<crossbeam::channel::Receiver<Vec<u8>>>,
    ) -> Result<bool, ServerError> {
        match message {
            Message::SubscribeAudio => {
                log::info!("🎤 {} subscribed to audio capture", client_id);

                // Initialize audio capture if not already done
                let was_initialized = {
                    let mut capture_guard = audio_capture.lock().unwrap();
                    if capture_guard.is_none() {
                        let capture = AudioCapture::new(config.audio_capture_config.clone())
                            .map_err(|e| ServerError::Audio(e.to_string()))?;
                        *capture_guard = Some(capture);
                        true
                    } else {
                        false
                    }
                };
                if was_initialized {
                    log::info!("🎤 Audio capture initialized");
                }

                // Create channel for this client and add to subscribers
                let (tx, rx) = crossbeam::channel::bounded(100);
                {
                    let mut subscribers = capture_subscribers.lock().unwrap();
                    subscribers.insert(client_id.to_string(), tx);
                }

                // Start the shared forwarding thread if not already running (atomic)
                if forwarding_thread_running
                    .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok()
                {
                    let audio_capture_clone = Arc::clone(audio_capture);
                    let capture_subscribers_clone = Arc::clone(capture_subscribers);
                    let forwarding_thread_running_clone = Arc::clone(forwarding_thread_running);

                    thread::spawn(move || {
                        Self::forward_audio_capture_shared(
                            audio_capture_clone,
                            capture_subscribers_clone,
                            forwarding_thread_running_clone,
                        );
                    });
                    log::info!("🎤 Started shared audio forwarding thread");
                }

                // Store the receiver for this client
                *audio_rx = Some(rx);

                Ok(true)
            }

            Message::PlayAudio {
                stream_id,
                audio_data,
            } => {
                log::debug!(
                    "🔊 Playing audio for stream: {} ({} bytes)",
                    stream_id,
                    audio_data.len()
                );

                // Initialize audio sink if not already done
                let was_initialized = {
                    let mut sink_guard = audio_sink.lock().unwrap();
                    if sink_guard.is_none() {
                        let sink = AudioSink::new(config.audio_sink_config.clone())
                            .map_err(|e| ServerError::Audio(e.to_string()))?;
                        *sink_guard = Some(sink);
                        true
                    } else {
                        false
                    }
                };
                if was_initialized {
                    log::info!("🔊 Audio sink initialized");
                }

                // Write audio data
                {
                    let sink_guard = audio_sink.lock().unwrap();
                    if let Some(ref sink) = *sink_guard {
                        sink.write_chunk(audio_data)
                            .map_err(|e| ServerError::Audio(e.to_string()))?;
                    }
                }

                // Send immediate response
                let response = Message::PlayResponse {
                    success: true,
                    message: "Audio chunk queued for playback".to_string(),
                };
                conn.write_message(&response)?;

                Ok(true)
            }

            Message::EndStream { stream_id } => {
                log::info!("⏹️  Ending audio stream: {}", stream_id);

                // Wait for playback completion
                {
                    let sink_guard = audio_sink.lock().unwrap();
                    if let Some(ref sink) = *sink_guard {
                        sink.end_stream_and_wait()
                            .map_err(|e| ServerError::Audio(e.to_string()))?;
                    }
                }

                let response = Message::EndStreamResponse {
                    success: true,
                    message: "Stream ended successfully".to_string(),
                    chunks_played: 0, // TODO: Track actual chunks played
                };
                conn.write_message(&response)?;

                Ok(true)
            }

            Message::AbortPlayback { stream_id } => {
                log::info!("🛑 Aborting playback: {}", stream_id);

                // Abort playback
                {
                    let sink_guard = audio_sink.lock().unwrap();
                    if let Some(ref sink) = *sink_guard {
                        sink.abort()
                            .map_err(|e| ServerError::Audio(e.to_string()))?;
                    }
                }

                let response = Message::AbortResponse {
                    success: true,
                    message: "Playback aborted successfully".to_string(),
                };
                conn.write_message(&response)?;

                Ok(true)
            }

            // These are server-to-client messages, shouldn't be received by server
            Message::AudioChunk { .. }
            | Message::PlayResponse { .. }
            | Message::EndStreamResponse { .. }
            | Message::AbortResponse { .. }
            | Message::ErrorResponse { .. } => {
                let error_msg = Message::ErrorResponse {
                    message: "Unexpected message type from client".to_string(),
                };
                conn.write_message(&error_msg)?;
                Ok(false) // Close connection
            }
        }
    }

    /// Shared thread that reads audio capture and distributes to all subscribers
    fn forward_audio_capture_shared(
        audio_capture: Arc<Mutex<Option<AudioCapture>>>,
        subscribers: Arc<Mutex<HashMap<String, crossbeam::channel::Sender<Vec<u8>>>>>,
        forwarding_thread_running: Arc<AtomicBool>,
    ) {
        log::info!("🎤 Audio forwarding thread started");
        let mut idle_count = 0;

        while forwarding_thread_running.load(Ordering::SeqCst) {
            // Check if we have any subscribers
            let has_subscribers = {
                let subscribers_guard = subscribers.lock().unwrap();
                !subscribers_guard.is_empty()
            };

            if !has_subscribers {
                // No subscribers, stop the thread
                forwarding_thread_running.store(false, Ordering::SeqCst);
                log::info!("🎤 No more subscribers, stopping forwarding thread");
                break;
            }

            let audio_data = {
                let capture_guard = audio_capture.lock().unwrap();
                match capture_guard.as_ref() {
                    Some(capture) => capture.try_next_chunk(),
                    None => {
                        thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                }
            };

            if let Some(data) = audio_data {
                // Reset idle counter - we have audio data
                idle_count = 0;

                // Send to all subscribers
                let mut to_remove = Vec::new();
                {
                    let subscribers_guard = subscribers.lock().unwrap();
                    for (client_id, sender) in subscribers_guard.iter() {
                        if sender.try_send(data.clone()).is_err() {
                            // Channel is full or closed
                            to_remove.push(client_id.clone());
                        }
                    }
                }

                // Remove dead subscribers
                if !to_remove.is_empty() {
                    {
                        let mut subscribers_guard = subscribers.lock().unwrap();
                        for client_id in &to_remove {
                            subscribers_guard.remove(client_id);
                        }
                    }
                    for client_id in to_remove {
                        log::debug!("🎤 Removed dead subscriber: {}", client_id);
                    }
                }
            } else {
                // No audio data available - use adaptive sleeping
                idle_count += 1;
                if idle_count > 5 {
                    // After being idle for a while, sleep longer to reduce CPU usage
                    thread::sleep(Duration::from_millis(2));
                }
                // Don't sleep on first few idle cycles to maintain low latency
            }
        }

        log::info!("🎤 Audio forwarding thread stopped");
    }
}
