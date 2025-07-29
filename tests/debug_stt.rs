//! Simple STT debugging test to isolate the hang issue

use std::thread;
use std::time::{Duration, Instant};

#[test]
fn test_stt_hang_debug() {
    env_logger::init();

    println!("🔍 Starting STT hang debug test");

    // Test 1: Can we create the basic services?
    println!("📋 Test 1: Creating basic services");

    let blocking_stt = agent::blocking_stt::BlockingSTTService::new("test-key".to_string());
    let mut stt_service = agent::services::stt::STTService::new(blocking_stt).unwrap();

    println!("✅ Services created successfully");

    // Test 2: Can we connect to a mock audio server?
    println!("📋 Test 2: Testing audio connection");

    // Start a simple mock server in background
    let server_handle = thread::spawn(|| {
        use std::io::Write;
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:12346").unwrap();
        println!("🎧 Mock audio server listening on 127.0.0.1:12346");

        for stream in listener.incoming() {
            match stream {
                Ok(mut stream) => {
                    println!("📡 Client connected to mock server");

                    // Send a few chunks of fake audio data
                    for i in 0..10 {
                        let chunk = vec![0u8; 1024]; // 1KB of silence
                        if stream.write_all(&chunk).is_err() {
                            break;
                        }
                        thread::sleep(Duration::from_millis(32)); // ~32ms per chunk
                        println!("📤 Sent chunk {}", i);
                    }

                    println!("✅ Mock server finished sending chunks");
                    break;
                }
                Err(e) => {
                    println!("❌ Mock server error: {}", e);
                    break;
                }
            }
        }
    });

    // Give server time to start
    thread::sleep(Duration::from_millis(100));

    // Test 3: Try to connect audio client (this might hang)
    println!("📋 Test 3: Connecting audio client");

    let start_time = Instant::now();

    // Try to connect with a timeout
    let connection_result = thread::spawn(move || {
        match audio_protocol::client::AudioClient::connect("127.0.0.1:12346") {
            Ok(mut client) => {
                println!("✅ Audio client connected");

                // Try to subscribe
                match client.subscribe_audio() {
                    Ok(_) => {
                        println!("✅ Audio subscription successful");
                        stt_service.set_audio_client(client);

                        // Try to start buffering
                        match stt_service.start_audio_buffering() {
                            Ok(_) => println!("✅ Audio buffering started"),
                            Err(e) => println!("❌ Audio buffering failed: {}", e),
                        }

                        // This is where it likely hangs - try transcription with timeout
                        println!("📋 Test 4: Attempting transcription (THIS MIGHT HANG)");
                        match stt_service.transcribe_from_wakeword() {
                            Ok(transcript) => {
                                println!("✅ Transcription successful: '{}'", transcript)
                            }
                            Err(e) => println!("❌ Transcription failed: {}", e),
                        }
                    }
                    Err(e) => println!("❌ Audio subscription failed: {}", e),
                }
            }
            Err(e) => println!("❌ Audio client connection failed: {}", e),
        }
    });

    // Wait for connection with timeout
    let timeout = Duration::from_secs(10);
    match connection_result.join() {
        Ok(_) => {
            let elapsed = start_time.elapsed();
            println!("✅ Test completed in {:.2}s", elapsed.as_secs_f32());
        }
    }

    // Clean up server
    server_handle.join().unwrap();

    println!("🏁 Debug test finished");
}
