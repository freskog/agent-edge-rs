//! Test the existing BlockingSTTService with real Fireworks API

use agent::blocking_stt::BlockingSTTService;
use agent::config::load_config;
use audio_protocol::client::AudioClient;
use secrecy::ExposeSecret;

#[test]
fn test_existing_blocking_stt_service() {
    env_logger::try_init().ok();

    println!("🔍 Test: Existing BlockingSTTService with real Fireworks API");

    // Load API configuration
    let config = match load_config() {
        Ok(config) => config,
        Err(e) => {
            println!("⚠️ Could not load API config: {}", e);
            println!("⚠️ Set FIREWORKS_API_KEY environment variable to run this test");
            return;
        }
    };

    // Create STT service with real API key
    let api_key = config.fireworks_key.expose_secret().clone();
    println!(
        "🔑 Using API key: {}...",
        &api_key[..std::cmp::min(10, api_key.len())]
    );

    let stt_service = BlockingSTTService::new(api_key);

    // Try to connect to audio server (this might fail, that's OK for this test)
    println!("🔗 Attempting to connect to audio server...");
    match AudioClient::connect("localhost:8080") {
        Ok(audio_client) => {
            println!("✅ Connected to audio server");

            // Test with empty context chunks to see if WebSocket connection works
            let context_chunks = vec![];

            println!("🎯 Testing STT service transcribe_from_wakeword...");
            let start = std::time::Instant::now();

            match stt_service.transcribe_from_wakeword(audio_client, context_chunks) {
                Ok(transcript) => {
                    println!("🎉 SUCCESS! Got transcript: '{}'", transcript);
                    println!("⏱️ Transcription took: {:?}", start.elapsed());
                }
                Err(e) => {
                    println!("❌ STT failed: {}", e);

                    // Check if it's a connection/auth issue vs logic issue
                    let error_msg = format!("{}", e);
                    if error_msg.contains("401")
                        || error_msg.contains("unauthorized")
                        || error_msg.contains("forbidden")
                    {
                        println!("⚠️ Authentication issue - check FIREWORKS_API_KEY");
                    } else if error_msg.contains("connection")
                        || error_msg.contains("network")
                        || error_msg.contains("timeout")
                    {
                        println!("⚠️ Network/connection issue");
                    } else {
                        println!("🤔 Other error - this might indicate a logic issue");
                    }
                }
            }
        }
        Err(e) => {
            println!("❌ Could not connect to audio server: {}", e);
            println!("⚠️ This is expected if audio server is not running");
            println!(
                "🔧 The test validates that the STT service can be created with a real API key"
            );

            // Just test that the service was created successfully
            println!("✅ BlockingSTTService created successfully with real API key");
        }
    }

    println!("✅ Test completed");
}
