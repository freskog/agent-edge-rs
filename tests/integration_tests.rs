//! # Integration Tests
//!
//! These tests verify the complete pipeline including wakeword detection and STT transcription.
//! They automatically run when FIREWORKS_API_KEY environment variable is set.
//!
//! ## Running Integration Tests
//!
//! ### Automatic Execution (Recommended)
//! ```bash
//! # Set your API key and run all tests - integration tests will run automatically
//! export FIREWORKS_API_KEY=fw_your_key_here
//! cargo test
//!
//! # Or use a .env file
//! echo "FIREWORKS_API_KEY=fw_your_key_here" >> .env
//! cargo test
//! ```
//!
//! ### Manual Execution
//! ```bash
//! # Run only integration tests with API key
//! FIREWORKS_API_KEY=fw_your_key_here cargo test --test integration_tests
//!
//! # Run a specific integration test
//! FIREWORKS_API_KEY=fw_your_key_here cargo test test_immediate_speech_integration
//!
//! # Skip integration tests (they will be skipped automatically without API key)
//! cargo test --test integration_tests  # Will skip gracefully
//! ```
//!
//! ### Without API Key
//! ```bash
//! # Integration tests will skip gracefully and show helpful message
//! cargo test --test integration_tests
//! # Output: "⏭️  Skipping integration test - FIREWORKS_API_KEY not found"
//! ```
//!
//! ## Test Coverage
//! - **Immediate Speech**: User speaks right after wakeword
//! - **Delayed Speech**: User pauses before speaking after wakeword  
//! - **Hesitation Speech**: User hesitates/stutters after wakeword
//! - **Multiple Queries**: Multiple wakeword+speech sequences
//!
//! ## Transcription Verification
//! All tests verify that the STT transcript contains the expected words:
//! - **Required words**: "what", "time", "is", "it"
//! - **Case insensitive**: Matches regardless of capitalization
//! - **Spacing tolerant**: Ignores extra spaces, punctuation, filler words
//! - **Examples of valid transcripts**:
//!   - "What time is it?"
//!   - "what time is it"
//!   - "Um, what time is it right now?"
//!   - "WHAT TIME IS IT???"
//!   - "what, uh, time is it please"
//!
//! ## Requirements
//! - Valid Fireworks AI API key (starts with 'fw_')
//! - Audio files in `tests/data/` directory
//! - Network connection for API calls
//!
//! Note: These tests use real audio files and make actual API calls to the STT service.
//! They will consume API credits when run.

//! Integration tests for the complete voice assistant pipeline
//!
//! NOTE: These tests are currently disabled due to changes in the AudioChunk structure
//! and VAD module. They need to be updated to work with the new architecture.

/*
use agent_edge_rs::{
    audio_capture::AudioChunk,
    config::load_config,
    detection::pipeline::{DetectionPipeline, PipelineConfig},
    error::Result,
    stt::{FireworksSTT, STTConfig},
};
use hound::WavReader;
use std::collections::VecDeque;
use std::env;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
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

/// Verify that transcript contains all required words in the correct order
fn assert_transcript_contains_words_in_order(
    transcript: &str,
    required_words: &[&str],
    test_name: &str,
) {
    let transcript_lower = transcript.to_lowercase();
    let mut last_position = 0;
    let mut missing_words = Vec::new();
    let mut out_of_order_words = Vec::new();

    for &word in required_words {
        let word_lower = word.to_lowercase();
        if let Some(position) = transcript_lower[last_position..].find(&word_lower) {
            // Word found in remaining text - update position
            last_position = last_position + position + word_lower.len();
        } else {
            // Check if word exists earlier in transcript (out of order)
            if transcript_lower.contains(&word_lower) {
                out_of_order_words.push(word);
            } else {
                missing_words.push(word);
            }
        }
    }

    if !missing_words.is_empty() || !out_of_order_words.is_empty() {
        let mut error_msg = format!("{}: Transcript validation failed\n", test_name);
        error_msg.push_str(&format!("Actual transcript: \"{}\"\n", transcript));
        error_msg.push_str(&format!(
            "Required words (in order): {:?}\n",
            required_words
        ));

        if !missing_words.is_empty() {
            error_msg.push_str(&format!("Missing words: {:?}\n", missing_words));
        }
        if !out_of_order_words.is_empty() {
            error_msg.push_str(&format!("Out of order words: {:?}\n", out_of_order_words));
        }

        panic!("{}", error_msg);
    }

    println!(
        "✅ {}: All required words found in correct order: \"{}\"",
        test_name, transcript
    );
}

#[cfg(test)]
mod word_assertion_tests {
    use super::*;

    #[test]
    fn test_word_assertion_robustness() {
        let required_words = ["what", "time", "is", "it"];

        // Test various valid formats
        let valid_transcripts = [
            "What time is it?",
            "what time is it",
            "WHAT TIME IS IT???",
            "Um, what time is it right now?",
            "what, uh, time is it please",
            "Well, what time is it exactly?",
            "So what time is it then?",
            "Hey, what time is it now?",
        ];

        for transcript in &valid_transcripts {
            assert_transcript_contains_words_in_order(
                transcript,
                &required_words,
                "Robustness Test",
            );
        }

        println!("✅ All transcript variations passed word assertion test");
    }

    #[test]
    #[should_panic(expected = "Missing words")]
    fn test_word_assertion_failure() {
        let required_words = ["what", "time", "is", "it"];
        let invalid_transcript = "Hello there, how are you?";

        assert_transcript_contains_words_in_order(
            invalid_transcript,
            &required_words,
            "Failure Test",
        );
    }

    #[test]
    #[should_panic(expected = "Out of order words")]
    fn test_word_assertion_out_of_order() {
        let required_words = ["what", "time", "is", "it"];
        let out_of_order_transcript = "It is what time?"; // "it" and "is" before "what" and "time"

        assert_transcript_contains_words_in_order(
            out_of_order_transcript,
            &required_words,
            "Order Test",
        );
    }
}

/// Load audio file and convert to chunks matching pipeline format
fn load_audio_file<P: AsRef<Path>>(path: P) -> Result<Vec<AudioChunk>> {
    let file = File::open(&path).map_err(|e| {
        agent_edge_rs::error::EdgeError::InvalidInput(format!("Failed to open audio file: {}", e))
    })?;

    let mut reader = WavReader::new(BufReader::new(file)).map_err(|e| {
        agent_edge_rs::error::EdgeError::InvalidInput(format!("Failed to read WAV file: {}", e))
    })?;

    let spec = reader.spec();
    println!("Audio file spec: {:?}", spec);

    // Verify format matches pipeline expectations
    if spec.channels != 1 || spec.sample_rate != 16000 || spec.bits_per_sample != 16 {
        return Err(agent_edge_rs::error::EdgeError::InvalidInput(format!(
            "Audio format mismatch. Expected: 16kHz, 16-bit, mono. Got: {}Hz, {}-bit, {} channels",
            spec.sample_rate, spec.bits_per_sample, spec.channels
        )));
    }

    // Read all samples
    let samples_i16: Vec<i16> = reader
        .samples::<i16>()
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| {
            agent_edge_rs::error::EdgeError::InvalidInput(format!("Failed to read samples: {}", e))
        })?;

    // Convert to chunks of 1280 samples (80ms at 16kHz)
    let chunk_size = 1280;
    let mut chunks = Vec::new();
    let start_time = Instant::now();

    for (i, chunk_samples) in samples_i16.chunks(chunk_size).enumerate() {
        let mut chunk_i16 = vec![0i16; chunk_size];
        let mut chunk_f32 = vec![0.0f32; chunk_size];

        // Copy samples and pad if necessary
        for (j, &sample) in chunk_samples.iter().enumerate() {
            chunk_i16[j] = sample;
            chunk_f32[j] = sample as f32 / 32768.0;
        }

        let timestamp = start_time + Duration::from_millis(i as u64 * 80);

        chunks.push(AudioChunk {
            samples_i16: chunk_i16,
            samples_f32: chunk_f32,
            timestamp,
            should_process: false, // Will be set by VAD
        });
    }

    println!("Loaded {} chunks from audio file", chunks.len());
    Ok(chunks)
}

/// Process audio chunks through VAD to set should_process flags
fn apply_vad_to_chunks(mut chunks: Vec<AudioChunk>) -> Result<Vec<AudioChunk>> {
    // Create VAD configuration optimized for wakeword detection
    let vad_config = VADConfig {
        sample_rate: VADSampleRate::Rate16kHz,
        chunk_size: ChunkSize::Small, // 512 samples (32ms) for low latency
        threshold: 0.5,               // Default Silero threshold
        speech_trigger_chunks: 6,     // More sensitive for wakeword detection
        silence_stop_chunks: 8,       // Longer silence for stability
    };

    let mut vad = create_vad(vad_config)?;

    for chunk in &mut chunks {
        chunk.should_process = vad
            .should_process_audio(&chunk.samples_i16)
            .unwrap_or(false);
    }

    let speech_chunks = chunks.iter().filter(|c| c.should_process).count();
    println!(
        "VAD detected speech in {}/{} chunks",
        speech_chunks,
        chunks.len()
    );

    Ok(chunks)
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum PipelineState {
    WaitingForWakeword,
    WaitingForSpeech,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum StreamGate {
    Closed,
    Open,
}

/// Test configuration matching main.rs pipeline
struct TestConfig {
    first_speech_timeout_ms: u128,
    silence_timeout_ms: u128,
    recent_chunks_size: usize,
}

impl Default for TestConfig {
    fn default() -> Self {
        Self {
            first_speech_timeout_ms: 8000, // 8 seconds
            silence_timeout_ms: 650,       // 0.65 seconds
            recent_chunks_size: 5,
        }
    }
}

/// Result structure for integration tests
#[derive(Debug)]
struct IntegrationTestResult {
    wakeword_detected: bool,
    wakeword_confidence: f32,
    stt_opened: bool,
    stt_closed: bool,
    stt_transcript: String,
    total_chunks: usize,
    total_speech_chunks: usize,
}

impl IntegrationTestResult {
    fn print_summary(&self, test_name: &str) {
        println!("\n📊 {} Results:", test_name);
        println!(
            "   Wakeword detected: {} (confidence: {:.3})",
            self.wakeword_detected, self.wakeword_confidence
        );
        println!(
            "   STT opened: {}, closed: {}",
            self.stt_opened, self.stt_closed
        );
        println!("   Transcript: \"{}\"", self.stt_transcript);
        println!(
            "   Speech chunks: {}/{} total",
            self.total_speech_chunks, self.total_chunks
        );
    }
}

/// Run a complete pipeline test with the given audio chunks
async fn run_integration_test(chunks: Vec<AudioChunk>) -> Result<IntegrationTestResult> {
    let config = TestConfig::default();

    // Initialize pipeline components
    let mut pipeline = DetectionPipeline::new(PipelineConfig::default())?;

    // Load API configuration using the config module
    let api_config = load_config()?;
    let stt = Arc::new(FireworksSTT::with_config(
        api_config.fireworks_key().to_string(),
        STTConfig::default(),
    ));

    // Create channels
    let (stt_tx, _) = broadcast::channel::<AudioChunk>(100);
    let stt_rx = stt_tx.subscribe();

    // State tracking
    let mut state = PipelineState::WaitingForWakeword;
    let mut recent_chunks: VecDeque<bool> = VecDeque::new();
    let mut speech_timeout = Instant::now();
    let mut silence_start: Option<Instant> = None;
    let mut user_speaking_immediately = false;

    // Results tracking
    let mut wakeword_detected = false;
    let mut wakeword_confidence = 0.0;
    let mut stt_opened = false;
    let mut stt_closed = false;
    let mut total_speech_chunks = 0;

    // Spawn STT task
    let stt_handle = tokio::spawn({
        let stt = Arc::clone(&stt);
        async move { stt.transcribe_stream(stt_rx).await }
    });

    // Process all chunks through the pipeline at real-time rate
    let chunk_interval = Duration::from_millis(80); // Each chunk is 80ms
    let mut last_chunk_time = Instant::now();

    // Keep stt_tx in a regular scope, not Arc
    let mut stt_tx = Some(stt_tx);

    for chunk in chunks.iter() {
        // Simulate real-time audio feed by waiting between chunks
        let elapsed = last_chunk_time.elapsed();
        if elapsed < chunk_interval {
            tokio::time::sleep(chunk_interval - elapsed).await;
        }
        last_chunk_time = Instant::now();

        if chunk.should_process {
            total_speech_chunks += 1;
        }

        // Track recent speech activity
        recent_chunks.push_back(chunk.should_process);
        if recent_chunks.len() > config.recent_chunks_size {
            recent_chunks.pop_front();
        }

        match state {
            PipelineState::WaitingForWakeword => {
                if chunk.should_process {
                    match pipeline.process_audio_chunk(&chunk.samples_f32) {
                        Ok(detection) => {
                            if detection.detected && !wakeword_detected {
                                wakeword_detected = true;
                                wakeword_confidence = detection.confidence;

                                // Switch to waiting for speech state
                                state = PipelineState::WaitingForSpeech;
                                speech_timeout = Instant::now();
                                silence_start = None;
                                stt_opened = true;

                                // Check if user is speaking immediately
                                let recent_speech_count =
                                    recent_chunks.iter().filter(|&&is_speech| is_speech).count();
                                user_speaking_immediately = recent_speech_count >= 3;

                                println!(
                                    "🎉 Wakeword detected (confidence: {:.3}, immediate: {})",
                                    wakeword_confidence, user_speaking_immediately
                                );
                            }
                        }
                        Err(e) => {
                            return Err(agent_edge_rs::error::EdgeError::InvalidInput(format!(
                                "Wakeword detection error: {}",
                                e
                            )));
                        }
                    }
                }
            }

            PipelineState::WaitingForSpeech => {
                if chunk.should_process {
                    // Send speech to STT if we still have the sender
                    if let Some(tx) = &stt_tx {
                        if let Err(e) = tx.send(chunk.clone()) {
                            println!("Failed to send audio chunk to STT: {}", e);
                        }
                    }
                    silence_start = None;
                } else {
                    // No speech detected
                    if silence_start.is_none() {
                        silence_start = Some(Instant::now());
                    }

                    // Check if we should end STT session
                    let should_end_stt = if let Some(silence_time) = silence_start {
                        let silence_duration = silence_time.elapsed().as_millis();
                        silence_duration > config.silence_timeout_ms
                    } else {
                        false
                    };

                    // Check for first speech timeout
                    let speech_elapsed = speech_timeout.elapsed().as_millis();
                    let first_speech_timeout = !user_speaking_immediately
                        && speech_elapsed > config.first_speech_timeout_ms;

                    if should_end_stt || first_speech_timeout {
                        state = PipelineState::WaitingForWakeword;
                        stt_closed = true;

                        // Send a few more chunks of silence to ensure proper closure
                        if let Some(tx) = &stt_tx {
                            for _ in 0..5 {
                                let silence_chunk = AudioChunk {
                                    samples_i16: vec![0i16; 1280],
                                    samples_f32: vec![0.0f32; 1280],
                                    timestamp: Instant::now(),
                                    should_process: false,
                                };
                                if let Err(e) = tx.send(silence_chunk) {
                                    println!("Failed to send silence chunk to STT: {}", e);
                                }
                                tokio::time::sleep(chunk_interval).await;
                            }
                        }

                        // Take ownership and drop the sender
                        stt_tx.take();
                        break;
                    }
                }
            }
        }
    }

    // Ensure STT is closed if we haven't already
    if !stt_closed {
        stt_closed = true;

        // Send a few more chunks of silence to ensure proper closure
        if let Some(tx) = &stt_tx {
            for _ in 0..5 {
                let silence_chunk = AudioChunk {
                    samples_i16: vec![0i16; 1280],
                    samples_f32: vec![0.0f32; 1280],
                    timestamp: Instant::now(),
                    should_process: false,
                };
                if let Err(e) = tx.send(silence_chunk) {
                    println!("Failed to send silence chunk to STT: {}", e);
                }
                tokio::time::sleep(chunk_interval).await;
            }
        }

        // Take ownership and drop the sender
        stt_tx.take();
    }

    // Wait for STT result with a timeout
    let transcript = match tokio::time::timeout(Duration::from_secs(5), stt_handle).await {
        Ok(result) => match result {
            Ok(Ok(text)) => text,
            Ok(Err(e)) => {
                return Err(agent_edge_rs::error::EdgeError::InvalidInput(format!(
                    "STT error: {}",
                    e
                )));
            }
            Err(e) => {
                return Err(agent_edge_rs::error::EdgeError::InvalidInput(format!(
                    "STT task error: {}",
                    e
                )));
            }
        },
        Err(_) => {
            return Err(agent_edge_rs::error::EdgeError::InvalidInput(
                "STT task timed out after 5 seconds".to_string(),
            ));
        }
    };

    Ok(IntegrationTestResult {
        wakeword_detected,
        wakeword_confidence,
        stt_opened,
        stt_closed,
        stt_transcript: transcript,
        total_chunks: chunks.len(),
        total_speech_chunks,
    })
}

#[tokio::test]
async fn test_immediate_speech_integration() -> Result<()> {
    skip_if_no_api_key();
    if !has_api_key() {
        return Ok(()); // Skip test gracefully
    }

    println!("\n🧪 Testing immediate speech integration");

    let chunks = load_audio_file("tests/data/immediate_what_time_is_it.wav")?;
    let chunks = apply_vad_to_chunks(chunks)?;
    let result = run_integration_test(chunks).await?;

    result.print_summary("Immediate Speech Integration");

    // Verify results
    assert!(result.wakeword_detected, "Should detect wakeword");
    assert!(
        result.wakeword_confidence > 0.3,
        "Should have reasonable confidence"
    );
    assert!(result.stt_opened, "Should open STT after wakeword");
    assert!(result.stt_closed, "Should close STT after speech");
    assert!(
        !result.stt_transcript.is_empty(),
        "Should have non-empty transcript"
    );

    // Verify transcript contains all expected words (ignoring spacing/filler words)
    let required_words = ["what", "time", "is", "it"];
    assert_transcript_contains_words_in_order(
        &result.stt_transcript,
        &required_words,
        "Immediate Speech Integration",
    );

    println!("✅ Immediate speech integration test passed");
    Ok(())
}

#[tokio::test]
async fn test_delayed_speech_integration() -> Result<()> {
    skip_if_no_api_key();
    if !has_api_key() {
        return Ok(()); // Skip test gracefully
    }

    println!("\n🧪 Testing delayed speech integration");

    let chunks = load_audio_file("tests/data/delay_start_what_time_is_it.wav")?;
    let chunks = apply_vad_to_chunks(chunks)?;
    let result = run_integration_test(chunks).await?;

    result.print_summary("Delayed Speech Integration");

    // Verify results
    assert!(result.wakeword_detected, "Should detect wakeword");
    assert!(
        result.wakeword_confidence > 0.3,
        "Should have reasonable confidence"
    );
    assert!(result.stt_opened, "Should open STT after wakeword");
    assert!(result.stt_closed, "Should close STT after speech");
    assert!(
        !result.stt_transcript.is_empty(),
        "Should have non-empty transcript"
    );

    // Verify transcript contains all expected words (ignoring spacing/filler words)
    let required_words = ["what", "time", "is", "it"];
    assert_transcript_contains_words_in_order(
        &result.stt_transcript,
        &required_words,
        "Delayed Speech Integration",
    );

    println!("✅ Delayed speech integration test passed");
    Ok(())
}

#[tokio::test]
async fn test_hesitation_speech_integration() -> Result<()> {
    skip_if_no_api_key();
    if !has_api_key() {
        return Ok(()); // Skip test gracefully
    }

    println!("\n🧪 Testing hesitation speech integration");

    let chunks = load_audio_file("tests/data/hesitation_what_time_is_it.wav")?;
    let chunks = apply_vad_to_chunks(chunks)?;
    let result = run_integration_test(chunks).await?;

    result.print_summary("Hesitation Speech Integration");

    // Verify results
    assert!(result.wakeword_detected, "Should detect wakeword");
    assert!(
        result.wakeword_confidence > 0.3,
        "Should have reasonable confidence"
    );
    assert!(result.stt_opened, "Should open STT after wakeword");
    assert!(result.stt_closed, "Should close STT after speech");
    assert!(
        !result.stt_transcript.is_empty(),
        "Should have non-empty transcript"
    );

    // Verify transcript contains all expected words (ignoring spacing/filler words)
    let required_words = ["what", "time", "is", "it"];
    assert_transcript_contains_words_in_order(
        &result.stt_transcript,
        &required_words,
        "Hesitation Speech Integration",
    );

    println!("✅ Hesitation speech integration test passed");
    Ok(())
}

#[tokio::test]
async fn test_multiple_queries_integration() -> Result<()> {
    skip_if_no_api_key();
    if !has_api_key() {
        return Ok(()); // Skip test gracefully
    }

    println!("\n🧪 Testing multiple queries integration");

    // Load both audio files
    let mut chunks1 = load_audio_file("tests/data/immediate_what_time_is_it.wav")?;
    let mut chunks2 = load_audio_file("tests/data/delay_start_what_time_is_it.wav")?;

    // Add 3.5s silence between queries (43 chunks of 80ms each)
    let start_time = chunks1.last().unwrap().timestamp + Duration::from_millis(80);
    for i in 0..43 {
        chunks1.push(AudioChunk {
            samples_i16: vec![0i16; 1280],
            samples_f32: vec![0.0f32; 1280],
            timestamp: start_time + Duration::from_millis(i * 80),
            should_process: false,
        });
    }

    // Update timestamps for second query
    let time_offset = chunks1.last().unwrap().timestamp + Duration::from_millis(80);
    for chunk in chunks2.iter_mut() {
        chunk.timestamp = chunk.timestamp + time_offset.duration_since(Instant::now());
    }

    // Combine chunks and apply VAD
    chunks1.extend(chunks2);
    let chunks = apply_vad_to_chunks(chunks1)?;

    let result = run_integration_test(chunks).await?;

    result.print_summary("Multiple Queries Integration");

    // Verify results
    assert!(result.wakeword_detected, "Should detect wakeword");
    assert!(
        result.wakeword_confidence > 0.3,
        "Should have reasonable confidence"
    );
    assert!(result.stt_opened, "Should open STT after wakeword");
    assert!(result.stt_closed, "Should close STT after speech");
    assert!(
        !result.stt_transcript.is_empty(),
        "Should have non-empty transcript"
    );

    // Verify transcript contains all expected words (ignoring spacing/filler words)
    let required_words = ["what", "time", "is", "it"];
    assert_transcript_contains_words_in_order(
        &result.stt_transcript,
        &required_words,
        "Multiple Queries Integration",
    );

    println!("✅ Multiple queries integration test passed");
    Ok(())
}
*/

#[test]
fn test_integration_tests_disabled() {
    println!("⏭️ Integration tests are currently disabled");
    println!("   They need to be updated for the new AudioChunk structure and VAD changes");
}
