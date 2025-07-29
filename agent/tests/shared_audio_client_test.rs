//! Tests for SharedAudioClient
//!
//! These tests verify that the SharedAudioClient properly:
//! 1. Connects to audio server and buffers audio
//! 2. Provides recent audio via message passing
//! 3. Streams live audio with proper EndMarker termination
//! 4. Handles timeouts and doesn't hang

use agent::audio::{AudioMessage, SharedAudioClient};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Mock audio server that implements the audio protocol properly
struct MockAudioServer {
    listener: TcpListener,
    should_stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl MockAudioServer {
    fn new(address: &str) -> Result<Self, std::io::Error> {
        let listener = TcpListener::bind(address)?;
        let should_stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

        Ok(Self {
            listener,
            should_stop,
        })
    }

    fn start(&self) -> std::thread::JoinHandle<()> {
        let listener = self.listener.try_clone().unwrap();
        let should_stop = self.should_stop.clone();

        thread::spawn(move || {
            println!("🎧 Mock audio server started");

            for stream in listener.incoming() {
                if should_stop.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }

                match stream {
                    Ok(mut stream) => {
                        println!("📡 Client connected to mock server");

                        // Handle client connection
                        if let Err(e) = Self::handle_client(&mut stream, &should_stop) {
                            println!("❌ Client handler error: {}", e);
                        }
                    }
                    Err(e) => {
                        println!("❌ Mock server error: {}", e);
                        break;
                    }
                }
            }

            println!("🔚 Mock audio server stopped");
        })
    }

    fn handle_client(
        stream: &mut TcpStream,
        should_stop: &std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Set a short read timeout so we can check should_stop periodically
        stream.set_read_timeout(Some(Duration::from_millis(100)))?;

        let mut subscribed = false;
        let mut chunk_count = 0u64;

        loop {
            if should_stop.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }

            // Try to read a message from client
            match Self::read_client_message(stream) {
                Ok(Some(message_type)) => {
                    println!("📨 Received message type: 0x{:02X}", message_type);

                    match message_type {
                        0x01 => {
                            // SubscribeAudio
                            subscribed = true;
                            println!("✅ Client subscribed to audio");
                        }
                        0x02 => {
                            // UnsubscribeAudio
                            subscribed = false;
                            println!("🔚 Client unsubscribed from audio");
                            break;
                        }
                        _ => {
                            println!("⚠️ Unhandled message type: 0x{:02X}", message_type);
                        }
                    }
                }
                Ok(None) => {
                    // No message available, continue
                }
                Err(e) => {
                    // Client likely disconnected
                    println!("📡 Client disconnected: {}", e);
                    break;
                }
            }

            // If subscribed, send audio chunks
            if subscribed {
                if let Err(e) = Self::send_audio_chunk(stream, chunk_count) {
                    println!("❌ Failed to send audio chunk: {}", e);
                    break;
                }
                chunk_count += 1;

                if chunk_count % 10 == 0 {
                    println!("📤 Sent {} audio chunks", chunk_count);
                }

                thread::sleep(Duration::from_millis(32)); // ~32ms per chunk
            } else {
                thread::sleep(Duration::from_millis(10));
            }
        }

        Ok(())
    }

    fn read_client_message(
        stream: &mut TcpStream,
    ) -> Result<Option<u8>, Box<dyn std::error::Error>> {
        let mut header = [0u8; 5]; // message_type + payload_length

        match stream.read_exact(&mut header) {
            Ok(_) => {
                let message_type = header[0];
                let payload_length =
                    u32::from_le_bytes([header[1], header[2], header[3], header[4]]);

                // Read and discard payload (we don't need it for this mock)
                if payload_length > 0 {
                    let mut payload = vec![0u8; payload_length as usize];
                    stream.read_exact(&mut payload)?;
                }

                Ok(Some(message_type))
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                Ok(None) // No data available
            }
            Err(e) => Err(e.into()),
        }
    }

    fn send_audio_chunk(
        stream: &mut TcpStream,
        chunk_id: u64,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Create mock audio data (1024 bytes = 512 samples at 16-bit)
        let audio_data = vec![0u8; 1024];

        // Get current timestamp
        let timestamp_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        // Build AudioChunk message according to protocol:
        // [message_type: u8][payload_length: u32][timestamp_ms: u64][audio_data_length: u32][audio_data...]
        let mut message = Vec::new();

        // Message type: AudioChunk = 0x10
        message.push(0x10);

        // Payload: timestamp_ms (8 bytes) + audio_data_length (4 bytes) + audio_data
        let payload_length = 8 + 4 + audio_data.len();
        message.extend_from_slice(&(payload_length as u32).to_le_bytes());

        // Timestamp
        message.extend_from_slice(&timestamp_ms.to_le_bytes());

        // Audio data length
        message.extend_from_slice(&(audio_data.len() as u32).to_le_bytes());

        // Audio data
        message.extend_from_slice(&audio_data);

        stream.write_all(&message)?;
        stream.flush()?;

        Ok(())
    }

    fn stop(&self) {
        self.should_stop
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

impl Drop for MockAudioServer {
    fn drop(&mut self) {
        self.stop();
    }
}

#[test]
fn test_shared_audio_client_basic_connection() {
    env_logger::try_init().ok(); // Initialize logging (ignore if already initialized)

    println!("🔍 Test: Basic SharedAudioClient connection");

    // Start mock server
    let server = MockAudioServer::new("127.0.0.1:12350").unwrap();
    let server_handle = server.start();

    // Give server time to start
    thread::sleep(Duration::from_millis(100));

    // Create SharedAudioClient
    let audio_client = SharedAudioClient::new("127.0.0.1:12350".to_string()).unwrap();

    // Wait a bit for connection and buffering
    thread::sleep(Duration::from_millis(500));

    // Check that we can get stats
    let stats = audio_client.get_stats().unwrap();
    println!("📊 Stats: {:?}", stats);

    // Should have received some chunks
    assert!(
        stats.total_chunks_received > 0,
        "Should have received audio chunks"
    );
    assert!(stats.is_connected, "Should be connected");
    assert!(stats.is_healthy(), "Should be healthy");

    // Clean up
    drop(audio_client);
    server.stop();
    server_handle.join().unwrap();

    println!("✅ Basic connection test passed");
}

#[test]
fn test_shared_audio_client_recent_audio() {
    env_logger::try_init().ok();

    println!("🔍 Test: Getting recent audio from buffer");

    // Start mock server
    let server = MockAudioServer::new("127.0.0.1:12351").unwrap();
    let server_handle = server.start();

    // Give server time to start
    thread::sleep(Duration::from_millis(100));

    // Create SharedAudioClient
    let audio_client = SharedAudioClient::new("127.0.0.1:12351".to_string()).unwrap();

    // Wait for some audio to be buffered
    thread::sleep(Duration::from_millis(1000)); // 1 second

    // Get recent audio (last 1 second)
    let recent_audio = audio_client.get_recent_audio(1).unwrap();
    println!("📥 Got {} chunks of recent audio", recent_audio.len());

    // Should have some audio chunks
    assert!(!recent_audio.is_empty(), "Should have recent audio chunks");

    // Each chunk should be reasonable size
    for chunk in &recent_audio {
        assert!(chunk.data.len() > 0, "Chunk should have data");
        assert!(chunk.data.len() <= 2048, "Chunk shouldn't be too large");
    }

    // Clean up
    drop(audio_client);
    server.stop();
    server_handle.join().unwrap();

    println!("✅ Recent audio test passed");
}

#[test]
fn test_shared_audio_client_live_stream() {
    env_logger::try_init().ok();

    println!("🔍 Test: Live audio streaming with EndMarker");

    // Start mock server
    let server = MockAudioServer::new("127.0.0.1:12352").unwrap();
    let server_handle = server.start();

    // Give server time to start
    thread::sleep(Duration::from_millis(100));

    // Create SharedAudioClient
    let audio_client = SharedAudioClient::new("127.0.0.1:12352".to_string()).unwrap();

    // Wait for connection
    thread::sleep(Duration::from_millis(200));

    // Start live stream for 500ms
    let stream_duration = Duration::from_millis(500);
    let stream_receiver = audio_client.start_live_stream(stream_duration).unwrap();

    let start_time = Instant::now();
    let mut chunk_count = 0;
    let mut got_end_marker = false;

    // Read from stream
    while let Ok(message) = stream_receiver.recv_timeout(Duration::from_secs(2)) {
        match message {
            AudioMessage::Chunk(chunk) => {
                chunk_count += 1;
                println!(
                    "📥 Received chunk {} ({} bytes)",
                    chunk_count,
                    chunk.data.len()
                );
                assert!(chunk.data.len() > 0, "Chunk should have data");
            }
            AudioMessage::EndMarker => {
                got_end_marker = true;
                println!(
                    "🔚 Received EndMarker after {}ms",
                    start_time.elapsed().as_millis()
                );
                break;
            }
        }

        // Safety timeout
        if start_time.elapsed() > Duration::from_secs(2) {
            panic!("Stream took too long - should have ended with EndMarker");
        }
    }

    // Verify we got chunks and EndMarker
    assert!(chunk_count > 0, "Should have received audio chunks");
    assert!(got_end_marker, "Should have received EndMarker");

    // Verify timing is approximately correct (within 200ms tolerance)
    let elapsed = start_time.elapsed();
    assert!(
        elapsed >= stream_duration,
        "Stream should last at least the requested duration"
    );
    assert!(
        elapsed < stream_duration + Duration::from_millis(200),
        "Stream shouldn't last much longer than requested"
    );

    // Clean up
    drop(audio_client);
    server.stop();
    server_handle.join().unwrap();

    println!("✅ Live stream test passed");
}

#[test]
fn test_shared_audio_client_no_infinite_loop() {
    env_logger::try_init().ok();

    println!("🔍 Test: No infinite loops - should terminate cleanly");

    // Start mock server
    let server = MockAudioServer::new("127.0.0.1:12353").unwrap();
    let server_handle = server.start();

    // Give server time to start
    thread::sleep(Duration::from_millis(100));

    let test_start = Instant::now();

    // Create and immediately drop SharedAudioClient
    {
        let audio_client = SharedAudioClient::new("127.0.0.1:12353".to_string()).unwrap();
        thread::sleep(Duration::from_millis(100)); // Brief usage
                                                   // Drop happens here
    }

    let elapsed = test_start.elapsed();
    println!("⏱️ Test completed in {}ms", elapsed.as_millis());

    // Should complete quickly (under 1 second)
    assert!(
        elapsed < Duration::from_secs(1),
        "SharedAudioClient should shut down quickly without hanging"
    );

    // Clean up
    server.stop();
    server_handle.join().unwrap();

    println!("✅ No infinite loop test passed");
}

#[test]
fn test_shared_audio_client_multiple_streams() {
    env_logger::try_init().ok();

    println!("🔍 Test: Multiple concurrent live streams");

    // Start mock server
    let server = MockAudioServer::new("127.0.0.1:12354").unwrap();
    let server_handle = server.start();

    // Give server time to start
    thread::sleep(Duration::from_millis(100));

    // Create SharedAudioClient
    let audio_client = SharedAudioClient::new("127.0.0.1:12354".to_string()).unwrap();

    // Wait for connection
    thread::sleep(Duration::from_millis(200));

    // Start two concurrent streams with different durations
    let stream1 = audio_client
        .start_live_stream(Duration::from_millis(300))
        .unwrap();
    let stream2 = audio_client
        .start_live_stream(Duration::from_millis(600))
        .unwrap();

    let start_time = Instant::now();
    let mut stream1_ended = false;
    let mut stream2_ended = false;
    let mut stream1_chunks = 0;
    let mut stream2_chunks = 0;

    // Read from both streams
    while !stream1_ended || !stream2_ended {
        // Check stream 1
        if !stream1_ended {
            match stream1.try_recv() {
                Ok(AudioMessage::Chunk(_)) => stream1_chunks += 1,
                Ok(AudioMessage::EndMarker) => {
                    stream1_ended = true;
                    println!(
                        "🔚 Stream 1 ended after {}ms with {} chunks",
                        start_time.elapsed().as_millis(),
                        stream1_chunks
                    );
                }
                Err(_) => {} // No message available
            }
        }

        // Check stream 2
        if !stream2_ended {
            match stream2.try_recv() {
                Ok(AudioMessage::Chunk(_)) => stream2_chunks += 1,
                Ok(AudioMessage::EndMarker) => {
                    stream2_ended = true;
                    println!(
                        "🔚 Stream 2 ended after {}ms with {} chunks",
                        start_time.elapsed().as_millis(),
                        stream2_chunks
                    );
                }
                Err(_) => {} // No message available
            }
        }

        // Safety timeout
        if start_time.elapsed() > Duration::from_secs(2) {
            panic!("Streams took too long to complete");
        }

        thread::sleep(Duration::from_millis(10));
    }

    // Verify both streams worked
    assert!(stream1_chunks > 0, "Stream 1 should have received chunks");
    assert!(stream2_chunks > 0, "Stream 2 should have received chunks");
    assert!(stream1_ended, "Stream 1 should have ended");
    assert!(stream2_ended, "Stream 2 should have ended");

    // Clean up
    drop(audio_client);
    server.stop();
    server_handle.join().unwrap();

    println!("✅ Multiple streams test passed");
}
