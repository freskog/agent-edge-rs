//! Simple test to verify SharedAudioClient basic functionality

use agent::audio::SharedAudioClient;
use std::time::{Duration, Instant};

#[test]
fn test_audio_client_creation_only() {
    println!("🔍 Testing SharedAudioClient creation and immediate drop");

    let start = Instant::now();

    // This should fail to connect (no server) but not hang
    match SharedAudioClient::new("127.0.0.1:99999".to_string()) {
        Ok(_) => {
            println!("❌ Unexpected success - no server should be running on port 99999");
            panic!("Expected connection failure");
        }
        Err(e) => {
            println!("✅ Expected connection failure: {}", e);
        }
    }

    let elapsed = start.elapsed();
    println!("⏱️ Test completed in {}ms", elapsed.as_millis());

    // Should fail quickly (under 5 seconds)
    assert!(
        elapsed < Duration::from_secs(5),
        "Connection attempt should fail quickly"
    );

    println!("✅ Creation test passed");
}
