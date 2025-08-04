use crate::audio_sink::{AudioSink, AudioSinkConfig};
use crate::protocol::{ProducerConnection, ProducerMessage, ProtocolError};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
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
}

impl ProducerServer {
    pub fn new(config: ProducerServerConfig) -> Self {
        Self {
            config,
            should_stop: Arc::new(AtomicBool::new(false)),
            producer_connected: Arc::new(AtomicBool::new(false)),
            audio_sink: Arc::new(Mutex::new(None)),
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

        // Signal handling
        let should_stop = Arc::clone(&self.should_stop);
        ctrlc::set_handler(move || {
            log::info!("🛑 Producer server received shutdown signal");
            should_stop.store(true, Ordering::SeqCst);
        })
        .ok();

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

        thread::spawn(move || {
            let result = Self::producer_thread(
                stream,
                addr.clone(),
                should_stop,
                producer_connected.clone(),
                audio_sink,
                sink_config,
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
    ) -> Result<(), ProducerServerError> {
        let mut connection = ProducerConnection::new(stream);

        // Send Connected confirmation immediately (no subscribe required for producer)
        connection.write_message(&ProducerMessage::Connected)?;
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

        while !should_stop.load(Ordering::SeqCst) {
            match connection.read_message() {
                Ok(message) => {
                    match message {
                        ProducerMessage::Play { data } => {
                            log::debug!("🔊 Received {} bytes of audio from producer", data.len());

                            // Send audio to sink
                            let sink_guard = audio_sink.lock().unwrap();
                            if let Some(sink) = sink_guard.as_ref() {
                                if let Err(e) = sink.write_chunk(data) {
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
                        ProducerMessage::Stop => {
                            log::info!("🛑 Producer {} requested stop (abort playback)", addr);

                            // Abort current playback and clear queue
                            let sink_guard = audio_sink.lock().unwrap();
                            if let Some(sink) = sink_guard.as_ref() {
                                if let Err(e) = sink.abort() {
                                    log::error!("❌ Failed to abort audio playback: {}", e);
                                    let error_msg = ProducerMessage::Error {
                                        message: format!("Failed to stop playback: {}", e),
                                    };
                                    connection.write_message(&error_msg)?;
                                } else {
                                    log::info!("✅ Audio playback stopped for producer {}", addr);
                                }
                            }
                        }
                        ProducerMessage::Connected | ProducerMessage::Error { .. } => {
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
