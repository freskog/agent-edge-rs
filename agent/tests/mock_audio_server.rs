use audio_protocol::protocol::{Connection, Message, ProtocolError};
use hound::WavReader;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

/// Mock audio server that serves test audio data for integration testing
pub struct MockAudioServer {
    listener: TcpListener,
    running: Arc<AtomicBool>,
    audio_data: Vec<Vec<u8>>, // Pre-loaded audio chunks
}

impl MockAudioServer {
    /// Create a new mock audio server with test audio data
    pub fn new(audio_file_path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        // Load and prepare audio data
        let audio_data = Self::load_audio_chunks(audio_file_path)?;

        // Bind to a random available port
        let listener = TcpListener::bind("127.0.0.1:0")?;

        println!(
            "Mock audio server created with {} audio chunks",
            audio_data.len()
        );

        Ok(Self {
            listener,
            running: Arc::new(AtomicBool::new(false)),
            audio_data,
        })
    }

    /// Get the address the server is bound to
    pub fn address(&self) -> std::io::Result<String> {
        let addr = self.listener.local_addr()?;
        Ok(format!("{}:{}", addr.ip(), addr.port()))
    }

    /// Start the server and return a handle to stop it
    pub fn start(&self) -> Result<MockServerHandle, Box<dyn std::error::Error>> {
        self.running.store(true, Ordering::SeqCst);

        let listener = self.listener.try_clone()?;
        let running = Arc::clone(&self.running);
        let audio_data = self.audio_data.clone();

        let handle = thread::spawn(move || {
            println!("🎙️ Mock audio server started");

            while running.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, addr)) => {
                        println!("📡 Client connected from {}", addr);
                        let client_running = Arc::clone(&running);
                        let client_audio_data = audio_data.clone();

                        thread::spawn(move || {
                            if let Err(e) =
                                Self::handle_client(stream, client_running, client_audio_data)
                            {
                                println!("❌ Client error: {}", e);
                            }
                        });
                    }
                    Err(e) => {
                        if running.load(Ordering::SeqCst) {
                            println!("❌ Accept error: {}", e);
                        }
                    }
                }
            }

            println!("🛑 Mock audio server stopped");
        });

        Ok(MockServerHandle {
            running: Arc::clone(&self.running),
            handle: Some(handle),
        })
    }

    /// Load audio file and convert to chunks matching the protocol format
    fn load_audio_chunks(file_path: &str) -> Result<Vec<Vec<u8>>, Box<dyn std::error::Error>> {
        let mut reader = WavReader::open(file_path)?;
        let spec = reader.spec();

        // Ensure correct format
        assert_eq!(spec.sample_rate, 16000, "Audio must be 16kHz");
        assert_eq!(spec.channels, 1, "Audio must be mono");

        let samples: Result<Vec<i16>, _> = reader.samples::<i16>().collect();
        let samples = samples?;

        let mut chunks = Vec::new();

        // Convert to 1280-sample chunks (80ms at 16kHz)
        for chunk_samples in samples.chunks(1280) {
            let mut chunk_data = Vec::new();

            // Convert i16 samples to bytes (little endian)
            for &sample in chunk_samples {
                chunk_data.extend_from_slice(&sample.to_le_bytes());
            }

            // Pad to 1280 samples if needed
            while chunk_data.len() < 1280 * 2 {
                chunk_data.extend_from_slice(&[0, 0]);
            }

            chunks.push(chunk_data);
        }

        // Add 5 seconds of silence at the end for reliable EOS detection
        // 5 seconds = 5000ms / 80ms per chunk = 62.5 chunks, so add 63 chunks
        let silence_chunks = 63;
        let mut silence_chunk_data = Vec::new();

        // Create silence chunk (1280 samples of zero as bytes)
        for _ in 0..1280 {
            silence_chunk_data.extend_from_slice(&[0, 0]); // i16 zero as bytes
        }

        for _ in 0..silence_chunks {
            chunks.push(silence_chunk_data.clone());
        }

        println!(
            "📄 Mock server: loaded {} original chunks + {} silence chunks = {} total chunks",
            chunks.len() - silence_chunks,
            silence_chunks,
            chunks.len()
        );

        Ok(chunks)
    }

    /// Handle a single client connection
    fn handle_client(
        stream: TcpStream,
        running: Arc<AtomicBool>,
        audio_data: Vec<Vec<u8>>,
    ) -> Result<(), ProtocolError> {
        let mut connection = Connection::new(stream)?;

        while running.load(Ordering::SeqCst) {
            // Try to read a message with simple timeout handling
            match connection.read_message() {
                Ok(message) => {
                    match message {
                        Message::SubscribeAudio => {
                            println!("🎧 Client subscribed to audio");

                            // Start streaming audio data
                            Self::stream_audio_data(&mut connection, &audio_data, &running)?;
                        }
                        Message::UnsubscribeAudio => {
                            println!("🔇 Client unsubscribed from audio");

                            let response = Message::UnsubscribeResponse {
                                success: true,
                                message: "Unsubscribed successfully".to_string(),
                            };
                            connection.write_message(&response)?;
                        }
                        other => {
                            println!("❓ Unhandled message: {:?}", other.message_type());
                            let error = Message::ErrorResponse {
                                message: format!(
                                    "Unsupported message type: {:?}",
                                    other.message_type()
                                ),
                            };
                            connection.write_message(&error)?;
                        }
                    }
                }
                Err(ProtocolError::Io(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    println!("📤 Client disconnected");
                    break;
                }
                Err(ProtocolError::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    // Non-blocking read returned nothing, wait briefly
                    thread::sleep(Duration::from_millis(10));
                    continue;
                }
                Err(e) => {
                    println!("❌ Protocol error: {}", e);
                    break;
                }
            }
        }

        Ok(())
    }

    /// Stream audio data to the client
    fn stream_audio_data(
        connection: &mut Connection,
        audio_data: &[Vec<u8>],
        running: &Arc<AtomicBool>,
    ) -> Result<(), ProtocolError> {
        println!("🎵 Starting audio stream ({} chunks)", audio_data.len());

        for (i, chunk_data) in audio_data.iter().enumerate() {
            if !running.load(Ordering::SeqCst) {
                break;
            }

            let message = Message::AudioChunk {
                audio_data: chunk_data.clone(),
                timestamp_ms: (i as u64) * 80, // 80ms per chunk
            };

            connection.write_message(&message)?;

            // Simulate real-time audio streaming (80ms per chunk)
            // BUT slow down to give transcription time to start
            let delay = if i < 50 {
                // First 50 chunks: slower to allow transcription to start
                Duration::from_millis(200) // 200ms per chunk instead of 80ms
            } else {
                // Rest: normal speed
                Duration::from_millis(80)
            };
            thread::sleep(delay);

            if i % 10 == 0 {
                println!("📡 Sent chunk {} of {}", i + 1, audio_data.len());
            }
        }

        println!("✅ Audio stream completed");

        // Keep the connection alive briefly
        thread::sleep(Duration::from_millis(500));

        Ok(())
    }
}

/// Handle to control the mock server
pub struct MockServerHandle {
    running: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl MockServerHandle {
    /// Stop the server
    pub fn stop(mut self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for MockServerHandle {
    fn drop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}
