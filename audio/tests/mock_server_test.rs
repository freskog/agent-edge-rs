//! Example test showing how to use MockAudioServer in tests
//!
//! This demonstrates the recommended pattern for integration tests
//! that need controlled audio input.

use audio::{MockAudioServer, MockServerConfig};
use audio_protocol::{AudioClient, Connection, Message};
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

#[test]
fn test_mock_server_basic_functionality() {
    env_logger::try_init().ok(); // Initialize logging (ignore if already initialized)

    // Configure mock server to use a test audio file
    let config = MockServerConfig {
        audio_file: PathBuf::from("../tests/data/hey_mycroft_test.wav"),
        bind_address: "127.0.0.1:0".to_string(), // Use random port
        loop_audio: false,                       // Play once for this test
        silence_duration: 1.0,
        speed: 4.0, // 4x speed for faster test
    };

    // Start the mock server in background
    let server_handle = MockAudioServer::new(config)
        .expect("Failed to create mock server")
        .start_background()
        .expect("Failed to start mock server");

    println!("🎵 Mock server started on port {}", server_handle.port);

    // Give server a moment to fully start
    thread::sleep(Duration::from_millis(100));

    // Connect as a client and subscribe to audio
    let mut client =
        AudioClient::new(&server_handle.address()).expect("Failed to connect to mock server");

    client.subscribe().expect("Failed to subscribe to audio");
    println!("📡 Client subscribed to mock server");

    // Receive some audio chunks
    let mut chunk_count = 0;
    let max_chunks = 10; // Stop after 10 chunks for this test

    while chunk_count < max_chunks {
        match client.read_chunk_timeout(Duration::from_secs(2)) {
            Ok(Some(chunk)) => {
                chunk_count += 1;
                println!(
                    "🎵 Received audio chunk {} ({} bytes)",
                    chunk_count,
                    chunk.data.len()
                );

                // Verify chunk format
                assert_eq!(chunk.data.len(), 2560); // 1280 samples * 2 bytes per sample
                assert!(chunk.timestamp_ms > 0);
            }
            Ok(None) => {
                println!("📄 End of audio stream");
                break;
            }
            Err(e) => {
                println!("⚠️ Error reading chunk: {}", e);
                break;
            }
        }
    }

    println!("✅ Test completed, received {} audio chunks", chunk_count);
    assert!(
        chunk_count > 0,
        "Should have received at least one audio chunk"
    );

    // Server will automatically stop when handle is dropped
}

#[test]
fn test_mock_server_multiple_clients() {
    env_logger::try_init().ok();

    let config = MockServerConfig {
        audio_file: PathBuf::from("../tests/data/immediate_what_time_is_it.wav"),
        bind_address: "127.0.0.1:0".to_string(),
        loop_audio: true,
        silence_duration: 0.5,
        speed: 8.0, // Very fast for quick test
    };

    let server_handle = MockAudioServer::new(config)
        .expect("Failed to create mock server")
        .start_background()
        .expect("Failed to start mock server");

    println!("🎵 Mock server started on port {}", server_handle.port);
    thread::sleep(Duration::from_millis(100));

    // Create multiple clients
    let mut clients = Vec::new();
    for i in 0..3 {
        let mut client =
            AudioClient::new(&server_handle.address()).expect("Failed to connect client");
        client.subscribe().expect("Failed to subscribe");
        clients.push(client);
        println!("📡 Client {} connected and subscribed", i);
    }

    // Each client receives chunks independently
    let mut handles = Vec::new();
    for (i, mut client) in clients.into_iter().enumerate() {
        let handle = thread::spawn(move || {
            let mut chunk_count = 0;
            for _ in 0..5 {
                if let Ok(Some(chunk)) = client.read_chunk_timeout(Duration::from_secs(1)) {
                    chunk_count += 1;
                    println!(
                        "🎵 Client {} received chunk {} ({} bytes)",
                        i,
                        chunk_count,
                        chunk.data.len()
                    );
                } else {
                    break;
                }
            }
            chunk_count
        });
        handles.push(handle);
    }

    // Wait for all clients to finish
    let mut total_chunks = 0;
    for (i, handle) in handles.into_iter().enumerate() {
        let chunks = handle.join().expect("Client thread panicked");
        println!("✅ Client {} received {} chunks", i, chunks);
        total_chunks += chunks;
    }

    println!(
        "✅ Multi-client test completed, total chunks: {}",
        total_chunks
    );
    assert!(
        total_chunks > 0,
        "Clients should have received audio chunks"
    );
}

#[test]
fn test_mock_server_with_different_files() {
    env_logger::try_init().ok();

    let test_files = vec![
        "../tests/data/hey_mycroft_test.wav",
        "../tests/data/alexa_test.wav",
        "../tests/data/immediate_what_time_is_it.wav",
    ];

    for (i, file_path) in test_files.iter().enumerate() {
        println!("🧪 Testing with file {}: {}", i + 1, file_path);

        let config = MockServerConfig {
            audio_file: PathBuf::from(file_path),
            bind_address: "127.0.0.1:0".to_string(),
            loop_audio: false,
            silence_duration: 0.5,
            speed: 8.0, // Very fast
        };

        let server_handle = MockAudioServer::new(config)
            .expect("Failed to create mock server")
            .start_background()
            .expect("Failed to start mock server");

        thread::sleep(Duration::from_millis(50));

        let mut client = AudioClient::new(&server_handle.address()).expect("Failed to connect");
        client.subscribe().expect("Failed to subscribe");

        // Read a few chunks to verify the file works
        let mut chunks_received = 0;
        for _ in 0..3 {
            if client
                .read_chunk_timeout(Duration::from_millis(500))
                .is_ok()
            {
                chunks_received += 1;
            }
        }

        println!(
            "✅ File {} worked, received {} chunks",
            file_path, chunks_received
        );
        assert!(
            chunks_received > 0,
            "Should receive audio from {}",
            file_path
        );

        // Server stops when handle is dropped
        drop(server_handle);
        thread::sleep(Duration::from_millis(50)); // Brief pause between tests
    }
}

/// Utility function that tests can use to start a mock server with default settings
#[allow(dead_code)]
pub fn start_test_mock_server() -> Result<audio::MockServerHandle, Box<dyn std::error::Error>> {
    let config = MockServerConfig {
        audio_file: PathBuf::from("../tests/data/hey_mycroft_test.wav"),
        bind_address: "127.0.0.1:0".to_string(),
        loop_audio: true,
        silence_duration: 1.0,
        speed: 2.0, // 2x speed for faster testing
    };

    MockAudioServer::new(config)?.start_background()
}

/// Example of how integration tests might use the mock server
#[test]
#[ignore] // Ignore by default since this is more of an example
fn example_integration_test_pattern() {
    env_logger::try_init().ok();

    // Start mock audio server
    let mock_server = start_test_mock_server().expect("Failed to start mock server");

    println!("🎵 Mock server running on {}", mock_server.address());

    // Here you would typically:
    // 1. Start your wakeword service pointing to mock_server.address()
    // 2. Start your agent service
    // 3. Perform test operations
    // 4. Verify expected behavior

    // For this example, just verify the server is working
    let mut client = AudioClient::new(&mock_server.address()).expect("Failed to connect");
    client.subscribe().expect("Failed to subscribe");

    let chunk = client
        .read_chunk_timeout(Duration::from_secs(2))
        .expect("Failed to read chunk")
        .expect("No chunk received");

    println!("✅ Received test chunk: {} bytes", chunk.data.len());

    // Mock server automatically stops when dropped
}
