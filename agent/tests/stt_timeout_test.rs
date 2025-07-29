//! Test that STT service has proper timeouts and doesn't hang

use agent::blocking_stt::BlockingSTTService;
use agent::services::stt::STTService;
use agent::services::STTService as STTServiceTrait; // Import the trait
use std::time::{Duration, Instant};

#[test]
fn test_stt_service_creation() {
    // Test 1: Can we create the services without hanging?
    println!("🔍 Testing STT service creation");

    let blocking_stt = BlockingSTTService::new("test-key".to_string());
    let _stt_service = STTService::new(blocking_stt).unwrap();

    println!("✅ STT services created successfully");

    // This test passes if we get here without hanging
    assert!(true);
}

#[test]
fn test_stt_no_audio_client() {
    // Test 2: What happens when we try to transcribe without audio client?
    println!("🔍 Testing STT without audio client");

    let blocking_stt = BlockingSTTService::new("test-key".to_string());
    let mut stt_service = STTService::new(blocking_stt).unwrap();

    // This should fail gracefully, not hang
    let start = Instant::now();
    let result = stt_service.transcribe_from_wakeword();
    let elapsed = start.elapsed();

    println!("⏱️ Transcription attempt took: {:?}", elapsed);

    // Should fail quickly (< 1 second) with a proper error
    assert!(elapsed < Duration::from_secs(1));
    assert!(result.is_err());

    println!("✅ Failed gracefully as expected");
}
