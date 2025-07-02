//! Audio sink integration tests

#![cfg(feature = "test-audio")]

use agent_edge_rs::{
    audio_sink::{AudioSink, CpalConfig, CpalSink},
    tts::{ElevenLabsTTS, TTSConfig},
};
use env_logger;
use std::{f32::consts::PI, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

/// Generate a sine wave at the specified frequency
fn generate_sine_wave(frequency: f32, duration_ms: u32) -> Vec<u8> {
    let sample_rate = 16000;
    let num_samples = (sample_rate as f32 * (duration_ms as f32 / 1000.0)) as usize;
    let mut samples = Vec::with_capacity(num_samples * 2);

    for i in 0..num_samples {
        let t = i as f32 / sample_rate as f32;
        let value = (2.0 * PI * frequency * t).sin();
        let sample = (value * i16::MAX as f32) as i16;
        samples.extend_from_slice(&sample.to_le_bytes());
    }

    samples
}

#[tokio::test]
async fn test_sine_wave_playback() {
    // Initialize logger
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Debug)
        .is_test(true)
        .try_init();

    println!("Playing 440 Hz sine wave for 2 seconds...");
    println!("You should hear a clear A4 note.");

    let sink = match CpalSink::new(CpalConfig::default()) {
        Ok(sink) => {
            println!("✅ Audio sink created successfully");
            sink
        }
        Err(e) => {
            println!("❌ Failed to create audio sink: {}", e);
            return;
        }
    };

    let samples = generate_sine_wave(440.0, 2000);
    println!("Generated {} bytes of audio data", samples.len());

    match sink.write(&samples).await {
        Ok(_) => println!("✅ Successfully wrote audio data to sink"),
        Err(e) => println!("❌ Failed to write audio data: {}", e),
    }

    println!("Waiting for audio to play...");
    tokio::time::sleep(Duration::from_millis(2500)).await;
    println!("Test complete");
}

#[tokio::test]
async fn test_silence_then_tone() {
    // Initialize logger
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Debug)
        .is_test(true)
        .try_init();

    println!("Playing 1 second of silence followed by 1 second 440 Hz tone...");
    println!("You should hear nothing for 1 second, then a clear A4 note.");

    let sink = match CpalSink::new(CpalConfig::default()) {
        Ok(sink) => {
            println!("✅ Audio sink created successfully");
            sink
        }
        Err(e) => {
            println!("❌ Failed to create audio sink: {}", e);
            return;
        }
    };

    // Generate 1 second of silence
    let silence = vec![0u8; 16000 * 2];
    println!("Writing {} bytes of silence...", silence.len());
    match sink.write(&silence).await {
        Ok(_) => println!("✅ Successfully wrote silence"),
        Err(e) => println!("❌ Failed to write silence: {}", e),
    }

    tokio::time::sleep(Duration::from_millis(1000)).await;

    // Generate 1 second of 440 Hz tone
    let samples = generate_sine_wave(440.0, 1000);
    println!("Writing {} bytes of tone...", samples.len());
    match sink.write(&samples).await {
        Ok(_) => println!("✅ Successfully wrote tone"),
        Err(e) => println!("❌ Failed to write tone: {}", e),
    }

    println!("Waiting for audio to complete...");
    tokio::time::sleep(Duration::from_millis(1500)).await;
    println!("Test complete");
}

#[tokio::test]
async fn test_tts_playback() {
    println!("Starting TTS playback test");
    env_logger::init();

    // Skip if no API key available
    let api_key = match std::env::var("ELEVENLABS_API_KEY") {
        Ok(key) => key,
        Err(_) => {
            println!("Skipping TTS test - ELEVENLABS_API_KEY not set");
            return;
        }
    };

    // Create audio sink with flexible config
    let config = CpalConfig::default();

    println!("Creating audio sink with flexible config...");
    let sink = match CpalSink::new(config) {
        Ok(sink) => {
            println!("✅ Audio sink created successfully");
            Arc::new(sink)
        }
        Err(e) => {
            println!("❌ Failed to create audio sink: {}", e);
            println!("Skipping TTS test due to audio device error");
            return;
        }
    };

    // Create TTS client
    let tts_config = TTSConfig {
        voice_id: "21m00Tcm4TlvDq8ikWAM".to_string(), // Rachel voice
        model: "eleven_monolingual_v1".to_string(),
        stability: 0.5,
        similarity_boost: 0.75,
        style: 0.0,
        use_speaker_boost: true,
    };
    let tts = ElevenLabsTTS::new(api_key, tts_config, sink);

    // Test text
    let text = "Testing text to speech playback.";
    println!("Synthesizing text: {}", text);

    // Synthesize and play
    let cancel = CancellationToken::new();
    match tts.synthesize(text, cancel).await {
        Ok(_) => println!("✅ TTS synthesis completed successfully"),
        Err(e) => {
            println!("❌ TTS synthesis failed: {}", e);
            panic!("TTS synthesis failed: {}", e);
        }
    }

    // Wait a bit for playback
    println!("Waiting for audio playback...");
    tokio::time::sleep(Duration::from_secs(5)).await;
    println!("Test complete");
}

#[tokio::test]
async fn test_rapid_writes() {
    // Initialize logger
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Debug)
        .is_test(true)
        .try_init();

    println!("Testing rapid audio writes...");
    println!("You should hear 5 short beeps in quick succession");

    let sink = match CpalSink::new(CpalConfig::default()) {
        Ok(sink) => {
            println!("✅ Audio sink created successfully");
            sink
        }
        Err(e) => {
            println!("❌ Failed to create audio sink: {}", e);
            return;
        }
    };

    let beep = generate_sine_wave(880.0, 100); // 100ms beep at A5
    println!("Generated beep of {} bytes", beep.len());

    for i in 0..5 {
        println!("Playing beep {} of 5...", i + 1);
        match sink.write(&beep).await {
            Ok(_) => println!("✅ Successfully wrote beep {}", i + 1),
            Err(e) => println!("❌ Failed to write beep {}: {}", i + 1, e),
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    println!("Waiting for final beep to complete...");
    tokio::time::sleep(Duration::from_millis(500)).await;
    println!("Test complete");
}
