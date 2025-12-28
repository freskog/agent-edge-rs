use crate::audio_sink::{AudioSink, AudioSinkConfig};
use crate::protocol::{ProducerConnection, ProducerMessage, ProtocolError};
use crossbeam::channel::Receiver;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ProducerServerError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Protocol error: {0}")]
    Protocol(#[from] ProtocolError),

    #[error("Audio error: {0}")]
    Audio(String),

    #[error("Producer already connected")]
    ProducerAlreadyConnected,
}

/// Configuration for the producer server
#[derive(Clone)]
pub struct ProducerServerConfig {
    pub bind_address: String,
    pub audio_sink_config: AudioSinkConfig,
}

impl Default for ProducerServerConfig {
    fn default() -> Self {
        Self {
            bind_address: "127.0.0.1:8081".to_string(),
            audio_sink_config: AudioSinkConfig::default(),
        }
    }
}

/// Producer server that accepts audio for playback from a single producer
pub struct ProducerServer {
    config: ProducerServerConfig,
    should_stop: Arc<AtomicBool>,
    producer_connected: Arc<AtomicBool>,
    audio_sink: Arc<Mutex<Option<AudioSink>>>,
    barge_in_rx: Option<Receiver<()>>, // Receives barge-in signals from consumer
}

impl ProducerServer {
    pub fn new(config: ProducerServerConfig) -> Self {
        Self {
            config,
            should_stop: Arc::new(AtomicBool::new(false)),
            producer_connected: Arc::new(AtomicBool::new(false)),
            audio_sink: Arc::new(Mutex::new(None)),
            barge_in_rx: None,
        }
    }

    /// Set the barge-in receiver (call before run())
    pub fn set_barge_in_receiver(&mut self, rx: Receiver<()>) {
        self.barge_in_rx = Some(rx);
    }

    /// Pre-initialize the audio sink before accepting connections
    /// This prevents audio loss on first connection
    pub fn initialize_sink(&self) -> Result<(), ProducerServerError> {
        let mut sink_guard = self.audio_sink.lock().unwrap();
        if sink_guard.is_none() {
            log::info!("🔊 Pre-initializing audio sink");
            match AudioSink::new(self.config.audio_sink_config.clone()) {
                Ok(sink) => {
                    *sink_guard = Some(sink);
                    log::info!("✅ Audio sink pre-initialized successfully");
                    Ok(())
                }
                Err(e) => {
                    Err(ProducerServerError::Audio(format!(
                        "Failed to pre-initialize audio sink: {}",
                        e
                    )))
                }
            }
        } else {
            Ok(())
        }
    }

    /// Start the producer server (blocking)
    pub fn run(&self) -> Result<(), ProducerServerError> {
        log::info!(
            "🔊 Starting Producer TCP server on {}",
            self.config.bind_address
        );

        let listener = TcpListener::bind(&self.config.bind_address)?;
        listener.set_nonblocking(true)?;

        log::info!(
            "🔊 Producer server listening on {}",
            self.config.bind_address
        );

        // Note: Signal handling is done in main.rs via stop() method

        while !self.should_stop.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((stream, addr)) => {
                    log::info!("🔊 Producer connection attempt from {}", addr);

                    // Check if we already have a producer
                    if self.producer_connected.load(Ordering::SeqCst) {
                        log::warn!("⚠️  Rejecting producer from {}: already connected", addr);
                        self.reject_producer(stream, "Producer already connected".to_string());
                        continue;
                    }

                    // Handle the producer connection
                    self.handle_producer(stream, addr.to_string());
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    // No connection available, sleep and continue
                    thread::sleep(Duration::from_millis(100));
                }
                Err(e) => {
                    log::error!("❌ Error accepting producer connection: {}", e);
                    thread::sleep(Duration::from_millis(1000));
                }
            }
        }

        log::info!("🛑 Producer server shutting down");
        Ok(())
    }

    /// Reject a producer connection with an error message
    fn reject_producer(&self, stream: TcpStream, error_message: String) {
        let mut connection = ProducerConnection::new(stream);
        let error_msg = ProducerMessage::Error {
            message: error_message,
        };

        if let Err(e) = connection.write_message(&error_msg) {
            log::error!(
                "❌ Failed to send error message to rejected producer: {}",
                e
            );
        }
        // Connection will be dropped when this function returns
    }

    /// Handle a single producer connection
    fn handle_producer(&self, stream: TcpStream, addr: String) {
        // Mark producer as connected
        self.producer_connected.store(true, Ordering::SeqCst);

        // Spawn thread to handle this producer
        let should_stop = Arc::clone(&self.should_stop);
        let producer_connected = Arc::clone(&self.producer_connected);
        let audio_sink = Arc::clone(&self.audio_sink);
        let sink_config = self.config.audio_sink_config.clone();
        let barge_in_rx = self.barge_in_rx.clone();

        thread::spawn(move || {
            let result = Self::producer_thread(
                stream,
                addr.clone(),
                should_stop,
                producer_connected.clone(),
                audio_sink,
                sink_config,
                barge_in_rx,
            );

            // Always mark producer as disconnected when thread exits
            producer_connected.store(false, Ordering::SeqCst);

            match result {
                Ok(()) => log::info!("✅ Producer {} disconnected cleanly", addr),
                Err(e) => log::error!("❌ Producer {} error: {}", addr, e),
            }
        });
    }

    /// Producer thread that handles the producer connection and plays audio
    fn producer_thread(
        stream: TcpStream,
        addr: String,
        should_stop: Arc<AtomicBool>,
        _producer_connected: Arc<AtomicBool>,
        audio_sink: Arc<Mutex<Option<AudioSink>>>,
        sink_config: AudioSinkConfig,
        barge_in_rx: Option<Receiver<()>>,
    ) -> Result<(), ProducerServerError> {
        let mut connection = ProducerConnection::new(stream);

        // No connection confirmation needed - client can start sending immediately
        log::info!("✅ Producer {} connected successfully", addr);

        // Initialize audio sink if not already running
        {
            let mut sink_guard = audio_sink.lock().unwrap();
            if sink_guard.is_none() {
                log::info!("🔊 Initializing audio sink for producer");
                match AudioSink::new(sink_config) {
                    Ok(sink) => {
                        *sink_guard = Some(sink);
                    }
                    Err(e) => {
                        let error_msg = ProducerMessage::Error {
                            message: format!("Failed to initialize audio sink: {}", e),
                        };
                        connection.write_message(&error_msg)?;
                        return Err(ProducerServerError::Audio(e.to_string()));
                    }
                }
            }
        }

        // Handle producer messages
        log::info!("🔊 Ready to receive audio from producer {}", addr);

        // Track stream state - stream IDs eliminate need for drain logic!
        let mut current_stream_id: u64 = 0; // 0 = idle, >0 = playing
        let mut interrupted_stream_id: u64 = 0; // Last interrupted stream
        let mut pending_completion: Option<mpsc::Receiver<()>> = None;

        while !should_stop.load(Ordering::SeqCst) {
            // Check for barge-in signal from consumer (wakeword detected during playback)
            if let Some(ref barge_in) = barge_in_rx {
                match barge_in.try_recv() {
                    Ok(()) => {
                        if current_stream_id != 0 {
                            log::info!(
                                "🔥 Barge-in interrupting stream {} for producer {}",
                                current_stream_id,
                                addr
                            );

                            // Mark this stream as interrupted
                            interrupted_stream_id = current_stream_id;
                            current_stream_id = 0;
                            pending_completion = None;

                            // Abort playback
                            let sink_guard = audio_sink.lock().unwrap();
                            if let Some(sink) = sink_guard.as_ref() {
                                if let Err(e) = sink.abort() {
                                    log::error!("❌ Failed to abort audio during barge-in: {}", e);
                                } else {
                                    log::info!("✅ Audio playback stopped due to barge-in");

                                    // Send PlaybackComplete to unblock client
                                    let complete_msg = ProducerMessage::PlaybackComplete {
                                        timestamp: ProducerMessage::current_timestamp(),
                                    };
                                    if let Err(e) = connection.write_message(&complete_msg) {
                                        log::error!(
                                            "❌ Failed to send PlaybackComplete after barge-in: {}",
                                            e
                                        );
                                        break;
                                    }
                                    log::info!("📤 Sent PlaybackComplete after barge-in (unblocking client)");
                                }
                            }
                        } else {
                            log::debug!("🔇 Barge-in signal received but no audio playing (ignored)");
                        }
                    }
                    Err(_) => {
                        // No barge-in signal, continue
                    }
                }
            }
            
            // Check if playback completion is ready (non-blocking)
            if let Some(ref completion_rx) = pending_completion {
                match completion_rx.try_recv() {
                    Ok(()) => {
                        log::info!(
                            "✅ Stream {} completed playback for producer {}",
                            current_stream_id,
                            addr
                        );
                        let complete_msg = ProducerMessage::PlaybackComplete {
                            timestamp: ProducerMessage::current_timestamp(),
                        };
                        if let Err(e) = connection.write_message(&complete_msg) {
                            log::error!("❌ Failed to send PlaybackComplete: {}", e);
                            break;
                        }
                        log::info!("📤 Sent PlaybackComplete, ready for next session");
                        pending_completion = None;
                        current_stream_id = 0; // Back to idle
                    }
                    Err(mpsc::TryRecvError::Empty) => {
                        // Still waiting for playback to complete
                    }
                    Err(mpsc::TryRecvError::Disconnected) => {
                        log::error!("❌ Completion signal lost");
                        pending_completion = None;
                        current_stream_id = 0;
                    }
                }
            }

            match connection.read_message() {
                Ok(message) => {
                    match message {
                        ProducerMessage::Play { data, stream_id } => {
                            log::debug!(
                                "🔊 Received {} bytes from stream {} (current: {}, interrupted: {})",
                                data.len(),
                                stream_id,
                                current_stream_id,
                                interrupted_stream_id
                            );

                            // Drop audio from old/interrupted streams
                            if stream_id <= interrupted_stream_id {
                                log::info!(
                                    "🗑️  Dropping {} bytes from old/interrupted stream {} (interrupted: {})",
                                    data.len(),
                                    stream_id,
                                    interrupted_stream_id
                                );
                                continue;
                            }

                            // Check if new stream is starting
                            if stream_id != current_stream_id {
                                log::info!(
                                    "🆕 Starting stream {} (previous: {})",
                                    stream_id,
                                    current_stream_id
                                );

                                // Drain any stale barge-in signals before starting new stream
                                if let Some(ref barge_in) = barge_in_rx {
                                    let mut drained = 0;
                                    while barge_in.try_recv().is_ok() {
                                        drained += 1;
                                    }
                                    if drained > 0 {
                                        log::info!("🧹 Drained {} stale barge-in signal(s) before starting new stream", drained);
                                    }
                                }

                                // NO ABORT NEEDED! Sink will handle stream switch atomically
                                // Old chunks will be dropped at buffer level without hardware reset
                                current_stream_id = stream_id;
                            }

                            // Send audio to sink WITH stream_id - sink handles switching!
                            let sink_guard = audio_sink.lock().unwrap();
                            if let Some(sink) = sink_guard.as_ref() {
                                if let Err(e) = sink.write_chunk(data, stream_id) {
                                    log::error!("❌ Failed to write audio to sink: {}", e);
                                    let error_msg = ProducerMessage::Error {
                                        message: format!("Audio playback error: {}", e),
                                    };
                                    connection.write_message(&error_msg)?;
                                }
                            } else {
                                log::error!("❌ Audio sink not initialized");
                                let error_msg = ProducerMessage::Error {
                                    message: "Audio sink not available".to_string(),
                                };
                                connection.write_message(&error_msg)?;
                            }
                        }
                        // Stop message removed - barge-in only stops server-side
                        ProducerMessage::EndOfStream {
                            timestamp,
                            stream_id,
                        } => {
                            // Only handle EndOfStream for current stream
                            if stream_id == current_stream_id {
                                log::info!(
                                    "🏁 Stream {} signaled end at timestamp {} for producer {}",
                                    stream_id,
                                    timestamp,
                                    addr
                                );

                                // Start non-blocking completion wait
                                let sink_guard = audio_sink.lock().unwrap();
                                if let Some(sink) = sink_guard.as_ref() {
                                    match sink.end_stream() {
                                        Ok(completion_rx) => {
                                            log::info!(
                                                "⏳ Monitoring playback completion for stream {} (non-blocking)",
                                                stream_id
                                            );
                                            pending_completion = Some(completion_rx);
                                        }
                                        Err(e) => {
                                            log::error!("❌ Failed to signal end of stream: {}", e);
                                            let error_msg = ProducerMessage::Error {
                                                message: format!("End of stream error: {}", e),
                                            };
                                            connection.write_message(&error_msg)?;
                                        }
                                    }
                                } else {
                                    log::warn!("⚠️  Audio sink not available for completion check");
                                }
                            } else {
                                log::info!(
                                    "🗑️  Ignoring EndOfStream for non-current stream {} (current: {})",
                                    stream_id,
                                    current_stream_id
                                );
                            }
                        }
                        ProducerMessage::Error { .. }
                        | ProducerMessage::PlaybackComplete { .. } => {
                            // These are server-to-client messages, should not be received
                            log::warn!(
                                "⚠️  Producer {} sent unexpected message: {:?}",
                                addr,
                                message
                            );
                        }
                    }
                }
                Err(ProtocolError::Io(ref e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    log::info!("🔌 Producer {} disconnected", addr);
                    break;
                }
                Err(ProtocolError::Io(ref e)) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    // No message available, sleep briefly
                    thread::sleep(Duration::from_millis(10));
                }
                Err(e) => {
                    log::error!("❌ Protocol error with producer {}: {}", addr, e);
                    let error_msg = ProducerMessage::Error {
                        message: format!("Protocol error: {}", e),
                    };
                    connection.write_message(&error_msg)?;
                    break;
                }
            }
        }

        log::info!("🛑 Producer connection ended for {}", addr);
        Ok(())
    }

    /// Stop the server
    pub fn stop(&self) {
        self.should_stop.store(true, Ordering::SeqCst);
    }
}
