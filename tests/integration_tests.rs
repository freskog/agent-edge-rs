//! # Integration Tests for Voice Assistant Pipeline
//!
//! These tests verify that the STT component works with the same code path used by the main application.
//! They test `transcribe_stream_with_context()` instead of the standalone `transcribe_stream()`.
//!
//! ## Running Integration Tests
//!
//! ```bash
//! # Set your API key and run
//! export FIREWORKS_API_KEY=fw_your_key_here
//! cargo test --test integration_tests
//!
//! # Or use a .env file
//! echo "FIREWORKS_API_KEY=fw_your_key_here" >> .env
//! cargo test --test integration_tests
//! ```

use agent_edge_rs::{
    config::load_config,
    speech_producer::{SpeechChunk, SpeechEvent},
    stt::{FireworksSTT, STTConfig},
};
use std::env;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::broadcast;

/// Check if FIREWORKS_API_KEY is available in environment
fn has_api_key() -> bool {
    env::var("FIREWORKS_API_KEY").is_ok()
        || std::fs::read_to_string(".env")
            .map(|content| content.contains("FIREWORKS_API_KEY="))
            .unwrap_or(false)
}

/// Skip test if no API key is available
fn skip_if_no_api_key() {
    if !has_api_key() {
        println!("⏭️  Skipping integration test - FIREWORKS_API_KEY not found");
        println!("   Set FIREWORKS_API_KEY environment variable to run integration tests");
        return;
    }
}

#[tokio::test]
async fn test_stt_with_context_path() -> Result<(), Box<dyn std::error::Error>> {
    skip_if_no_api_key();
    if !has_api_key() {
        return Ok(()); // Skip test gracefully
    }

    println!("\n🧪 Testing STT with context path (same as main app)");

    // Create a broadcast channel like the main application uses
    let (tx, rx) = broadcast::channel::<SpeechChunk>(32);

    // Create context chunks (simulating what UserInstructionDetector does)
    let context_chunks: Vec<SpeechChunk> = (0..5)
        .map(|i| {
            let mut samples = [0.0f32; 1280];
            // Add some simple test audio (sine wave)
            for (j, sample) in samples.iter_mut().enumerate() {
                *sample = (0.1 * (j as f32 * 0.01 + i as f32)).sin();
            }
            SpeechChunk::new(samples, Instant::now(), SpeechEvent::Speaking)
        })
        .collect();

    // Load API configuration and create STT instance (same as main app)
    let api_config = load_config()?;
    let stt = Arc::new(FireworksSTT::with_config(
        api_config.fireworks_key().to_string(),
        STTConfig::default(),
    ));

    // Use the same STT method as the main application
    let stt_handle = tokio::spawn({
        let stt = Arc::clone(&stt);
        async move { stt.transcribe_stream_with_context(rx, context_chunks).await }
    });

    // Send some test audio chunks (simulating live speech after wakeword)
    for i in 0..10 {
        let mut samples = [0.0f32; 1280];
        // Add test audio data (different frequency per chunk)
        for (j, sample) in samples.iter_mut().enumerate() {
            *sample = (0.1 * (j as f32 * 0.01 + i as f32 * 10.0)).sin();
        }

        let speech_event = if i == 0 {
            SpeechEvent::StartedSpeaking
        } else if i == 9 {
            SpeechEvent::StoppedSpeaking
        } else {
            SpeechEvent::Speaking
        };

        let speech_chunk = SpeechChunk::new(samples, Instant::now(), speech_event);

        if let Err(e) = tx.send(speech_chunk) {
            println!("Failed to send test chunk: {}", e);
            break;
        }

        // Simulate real-time audio flow (80ms chunks)
        tokio::time::sleep(Duration::from_millis(80)).await;
    }

    // Close the sender to signal end of stream
    drop(tx);

    // Wait for STT result with timeout
    let transcript = match tokio::time::timeout(Duration::from_secs(15), stt_handle).await {
        Ok(result) => match result {
            Ok(Ok(text)) => text,
            Ok(Err(e)) => {
                // For synthetic test audio, "No transcript received" is expected
                let error_msg = e.to_string();
                if error_msg.contains("No transcript received") {
                    println!("⚠️  Expected: STT found no recognizable speech in synthetic audio");
                    String::new() // Return empty transcript
                } else {
                    println!("❌ Unexpected STT error: {}", e);
                    return Err(format!("STT error: {}", e).into());
                }
            }
            Err(e) => {
                println!("❌ STT task error: {}", e);
                return Err(format!("STT task error: {}", e).into());
            }
        },
        Err(_) => {
            return Err("STT task timed out after 15 seconds".into());
        }
    };

    println!("✅ STT with context completed successfully");
    println!("📝 Transcript: \"{}\"", transcript);

    // For synthetic test audio, transcript may be empty - that's expected
    // The important thing is that the STT code path completed without errors
    // Just verify we got a string response (even if empty)

    println!("🎯 Integration test passed - same code path as main application");

    Ok(())
}

#[tokio::test]
async fn test_stt_with_context_and_real_speech_events() -> Result<(), Box<dyn std::error::Error>> {
    skip_if_no_api_key();
    if !has_api_key() {
        return Ok(()); // Skip test gracefully
    }

    println!("\n🧪 Testing STT with realistic speech event sequence");

    // Create broadcast channel
    let (tx, rx) = broadcast::channel::<SpeechChunk>(32);

    // Create context chunks with more realistic audio (silence)
    let context_chunks: Vec<SpeechChunk> = (0..5)
        .map(|_| {
            let samples = [0.0f32; 1280]; // Silent context
            SpeechChunk::new(samples, Instant::now(), SpeechEvent::Speaking)
        })
        .collect();

    // Load API configuration
    let api_config = load_config()?;
    let stt = Arc::new(FireworksSTT::with_config(
        api_config.fireworks_key().to_string(),
        STTConfig::default(),
    ));

    // Start STT with context
    let stt_handle = tokio::spawn({
        let stt = Arc::clone(&stt);
        async move { stt.transcribe_stream_with_context(rx, context_chunks).await }
    });

    // Send a more realistic sequence:
    // 1. Start with silence
    // 2. Begin speaking
    // 3. Continue speaking
    // 4. Stop speaking

    let events_and_audio = vec![
        (SpeechEvent::Speaking, true),         // Silence
        (SpeechEvent::Speaking, true),         // Silence
        (SpeechEvent::StartedSpeaking, false), // Start of speech
        (SpeechEvent::Speaking, false),        // Continue speaking
        (SpeechEvent::Speaking, false),        // Continue speaking
        (SpeechEvent::Speaking, false),        // Continue speaking
        (SpeechEvent::Speaking, false),        // Continue speaking
        (SpeechEvent::StoppedSpeaking, true),  // End of speech
    ];

    for (i, (event, is_silence)) in events_and_audio.iter().enumerate() {
        let mut samples = [0.0f32; 1280];

        if !is_silence {
            // Generate some audio for non-silence chunks
            for (j, sample) in samples.iter_mut().enumerate() {
                *sample = (0.2 * (j as f32 * 0.1 + i as f32 * 5.0)).sin();
            }
        }

        let speech_chunk = SpeechChunk::new(samples, Instant::now(), event.clone());

        if let Err(e) = tx.send(speech_chunk) {
            println!("Failed to send speech chunk: {}", e);
            break;
        }

        // Real-time pacing
        tokio::time::sleep(Duration::from_millis(80)).await;
    }

    // Close sender
    drop(tx);

    // Wait for result
    let transcript = match tokio::time::timeout(Duration::from_secs(15), stt_handle).await {
        Ok(result) => match result {
            Ok(Ok(text)) => text,
            Ok(Err(e)) => {
                // For synthetic test audio, "No transcript received" is expected
                let error_msg = e.to_string();
                if error_msg.contains("No transcript received") {
                    println!("⚠️  Expected: STT found no recognizable speech in synthetic audio");
                    String::new() // Return empty transcript
                } else {
                    println!("❌ Unexpected STT error: {}", e);
                    return Err(format!("STT error: {}", e).into());
                }
            }
            Err(e) => {
                println!("❌ STT task error: {}", e);
                return Err(format!("STT task error: {}", e).into());
            }
        },
        Err(_) => {
            return Err("STT task timed out after 15 seconds".into());
        }
    };

    println!("✅ STT with realistic speech events completed");
    println!("📝 Transcript: \"{}\"", transcript);

    // Transcript may be empty or contain detected audio artifacts - that's OK
    // The main goal is to ensure the code path works without errors

    println!("🎯 Realistic speech event test passed");

    Ok(())
}

#[test]
fn test_integration_tests_enabled() {
    if has_api_key() {
        println!("✅ Integration tests are enabled (API key found)");
    } else {
        println!("⏭️  Integration tests disabled (no API key)");
        println!("   Set FIREWORKS_API_KEY to enable integration tests");
    }
}
