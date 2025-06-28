//! # STT Pipeline Integration Tests
//!
//! Tests for the STT pipeline state machine, timing logic, and audio flow.
//! These tests simulate the main pipeline behavior without requiring actual
//! microphone input or network calls.

use agent_edge_rs::{AudioChunk, error::Result};
use std::collections::VecDeque;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, mpsc};
use tokio::time::timeout;

/// Simulates the pipeline state machine logic from main.rs
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

/// Test configuration for timing parameters
struct TestConfig {
    first_speech_timeout_ms: u128,
    silence_timeout_ms: u128,
    recent_chunks_size: usize,
}

impl Default for TestConfig {
    fn default() -> Self {
        Self {
            first_speech_timeout_ms: 8000,
            silence_timeout_ms: 650,
            recent_chunks_size: 5,
        }
    }
}

/// Simulates the main pipeline state machine logic
struct PipelineSimulator {
    config: TestConfig,
    state: PipelineState,
    recent_chunks: VecDeque<bool>,
    speech_timeout: Instant,
    silence_start: Option<Instant>,
    user_speaking_immediately: bool,
}

impl PipelineSimulator {
    fn new(config: TestConfig) -> Self {
        Self {
            config,
            state: PipelineState::WaitingForWakeword,
            recent_chunks: VecDeque::new(),
            speech_timeout: Instant::now(),
            silence_start: None,
            user_speaking_immediately: false,
        }
    }

    /// Simulates wakeword detection and state transition
    fn detect_wakeword(&mut self) -> StreamGate {
        // Check if user is speaking immediately after wakeword
        let recent_speech_count = self.recent_chunks.iter().filter(|&&is_speech| is_speech).count();
        let immediate_speech = recent_speech_count >= 3;

        self.state = PipelineState::WaitingForSpeech;
        self.speech_timeout = Instant::now();
        self.silence_start = None;
        self.user_speaking_immediately = immediate_speech;

        println!("🎉 Wakeword detected! Immediate speech: {}", immediate_speech);
        StreamGate::Open
    }

    /// Processes an audio chunk and returns whether STT should continue
    fn process_chunk(&mut self, has_speech: bool) -> Option<StreamGate> {
        // Track recent speech activity
        self.recent_chunks.push_back(has_speech);
        if self.recent_chunks.len() > self.config.recent_chunks_size {
            self.recent_chunks.pop_front();
        }

        match self.state {
            PipelineState::WaitingForWakeword => {
                // In real implementation, this would run wakeword detection
                None
            }
            PipelineState::WaitingForSpeech => {
                if has_speech {
                    // Speech detected - reset silence timer
                    self.silence_start = None;
                    None
                } else {
                    // No speech detected
                    if self.silence_start.is_none() {
                        self.silence_start = Some(Instant::now());
                    }

                    // Check if we should end STT session
                    let should_end_stt = if let Some(silence_time) = self.silence_start {
                        let silence_duration = silence_time.elapsed().as_millis();
                        silence_duration > self.config.silence_timeout_ms
                    } else {
                        false
                    };

                    // Check for first speech timeout, but only if user didn't speak immediately
                    let speech_elapsed = self.speech_timeout.elapsed().as_millis();
                    let first_speech_timeout = !self.user_speaking_immediately 
                        && speech_elapsed > self.config.first_speech_timeout_ms;

                    if should_end_stt || first_speech_timeout {
                        self.state = PipelineState::WaitingForWakeword;
                        let reason = if should_end_stt { "silence" } else { "timeout" };
                        println!("🔇 Ending STT session due to {}", reason);
                        Some(StreamGate::Closed)
                    } else {
                        None
                    }
                }
            }
        }
    }

    fn get_state(&self) -> PipelineState {
        self.state
    }
}

/// Creates test audio chunks with speech/silence patterns
fn create_test_chunks(pattern: &[bool], chunk_duration_ms: u64) -> Vec<(AudioChunk, bool)> {
    pattern.iter().enumerate().map(|(i, &has_speech)| {
        let timestamp = Instant::now() + Duration::from_millis(i as u64 * chunk_duration_ms);
        let audio_chunk = AudioChunk {
            samples_i16: vec![if has_speech { 1000 } else { 0 }; 1280], // 80ms at 16kHz
            samples_f32: vec![if has_speech { 0.1 } else { 0.0 }; 1280],
            timestamp,
            should_process: has_speech,
        };
        (audio_chunk, has_speech)
    }).collect()
}

#[tokio::test]
async fn test_immediate_speech_after_wakeword() -> Result<()> {
    println!("🧪 Testing immediate speech after wakeword");
    
    let config = TestConfig::default();
    let mut simulator = PipelineSimulator::new(config);

    // Simulate speech pattern: [speech, speech, speech, silence, silence, ...]
    // This represents "hey mycroft what time is it"
    let speech_pattern = vec![true, true, true, true, true, false, false, false, false, false];
    let chunks = create_test_chunks(&speech_pattern, 80);

    // Process chunks to build recent speech history
    for (_, has_speech) in &chunks[..5] {
        simulator.process_chunk(*has_speech);
    }

    // Detect wakeword (should detect immediate speech)
    let gate = simulator.detect_wakeword();
    assert_eq!(gate, StreamGate::Open);
    assert_eq!(simulator.get_state(), PipelineState::WaitingForSpeech);

    // Continue processing - should handle speech and then silence
    let mut stt_closed = false;
    for (_, has_speech) in &chunks[5..] {
        if let Some(StreamGate::Closed) = simulator.process_chunk(*has_speech) {
            stt_closed = true;
            break;
        }
    }

    assert!(stt_closed, "STT should close after silence period");
    assert_eq!(simulator.get_state(), PipelineState::WaitingForWakeword);
    
    println!("✅ Immediate speech test passed");
    Ok(())
}

#[tokio::test]
async fn test_paused_speech_after_wakeword() -> Result<()> {
    println!("🧪 Testing paused speech after wakeword");
    
    let config = TestConfig::default();
    let mut simulator = PipelineSimulator::new(config);

    // Simulate speech pattern: [speech, silence, silence, speech, speech, ...]
    // This represents "hey mycroft... what time is it"
    let speech_pattern = vec![true, false, false, false, false, true, true, true, false, false];
    let chunks = create_test_chunks(&speech_pattern, 80);

    // Process chunks to build recent speech history (mostly silence)
    for (_, has_speech) in &chunks[..5] {
        simulator.process_chunk(*has_speech);
    }

    // Detect wakeword (should NOT detect immediate speech)
    let gate = simulator.detect_wakeword();
    assert_eq!(gate, StreamGate::Open);
    assert_eq!(simulator.get_state(), PipelineState::WaitingForSpeech);

    // Continue processing - should wait for speech, then handle silence
    let mut stt_closed = false;
    for (_, has_speech) in &chunks[5..] {
        if let Some(StreamGate::Closed) = simulator.process_chunk(*has_speech) {
            stt_closed = true;
            break;
        }
    }

    assert!(stt_closed, "STT should close after silence period");
    assert_eq!(simulator.get_state(), PipelineState::WaitingForWakeword);
    
    println!("✅ Paused speech test passed");
    Ok(())
}

#[tokio::test]
async fn test_first_speech_timeout() -> Result<()> {
    println!("🧪 Testing first speech timeout");
    
    let mut config = TestConfig::default();
    config.first_speech_timeout_ms = 100; // Very short timeout for testing
    let mut simulator = PipelineSimulator::new(config);

    // Simulate no speech after wakeword
    let speech_pattern = vec![false, false, false, false, false];
    let chunks = create_test_chunks(&speech_pattern, 80);

    // Process chunks to build recent speech history (no speech)
    for (_, has_speech) in &chunks {
        simulator.process_chunk(*has_speech);
    }

    // Detect wakeword (should NOT detect immediate speech)
    simulator.detect_wakeword();

    // Wait for timeout
    tokio::time::sleep(Duration::from_millis(150)).await;

    // Process one more chunk - should trigger timeout
    let result = simulator.process_chunk(false);
    assert_eq!(result, Some(StreamGate::Closed));
    assert_eq!(simulator.get_state(), PipelineState::WaitingForWakeword);
    
    println!("✅ First speech timeout test passed");
    Ok(())
}

#[tokio::test]
async fn test_silence_gap_timing() -> Result<()> {
    println!("🧪 Testing silence gap timing");
    
    let mut config = TestConfig::default();
    config.silence_timeout_ms = 200; // Short timeout for testing
    let mut simulator = PipelineSimulator::new(config);

    // Build immediate speech history
    for _ in 0..5 {
        simulator.process_chunk(true);
    }

    // Detect wakeword
    simulator.detect_wakeword();

    // Send some speech
    simulator.process_chunk(true);
    simulator.process_chunk(true);

    // Start silence
    let start_time = Instant::now();
    
    // Process silence chunks until timeout
    let mut stt_closed = false;
    while start_time.elapsed().as_millis() < 300 {
        if let Some(StreamGate::Closed) = simulator.process_chunk(false) {
            stt_closed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    assert!(stt_closed, "STT should close after silence timeout");
    
    // Check timing is approximately correct (should be ~200ms)
    let elapsed = start_time.elapsed().as_millis();
    assert!(elapsed >= 200 && elapsed < 350, "Silence timeout should be ~200ms, got {}ms", elapsed);
    
    println!("✅ Silence gap timing test passed ({}ms)", elapsed);
    Ok(())
}

#[tokio::test]
async fn test_stt_channel_flow() -> Result<()> {
    println!("🧪 Testing STT channel flow");
    
    let (audio_tx, mut audio_rx) = mpsc::channel::<AudioChunk>(10);
    let (stt_tx, mut stt_rx) = broadcast::channel::<AudioChunk>(10);
    let (control_tx, mut control_rx) = mpsc::channel::<StreamGate>(10);

    // Simulate sending audio chunks
    let chunks = create_test_chunks(&[true, true, false, false], 80);
    
    // Spawn task to simulate pipeline behavior
    let stt_tx_clone = stt_tx.clone();
    let handle = tokio::spawn(async move {
        let mut state = PipelineState::WaitingForWakeword;
        
        while let Some(audio_chunk) = audio_rx.recv().await {
            match state {
                PipelineState::WaitingForWakeword => {
                    if audio_chunk.should_process {
                        // Simulate wakeword detection
                        state = PipelineState::WaitingForSpeech;
                        control_tx.send(StreamGate::Open).await.unwrap();
                    }
                }
                PipelineState::WaitingForSpeech => {
                    if audio_chunk.should_process {
                        stt_tx_clone.send(audio_chunk).unwrap();
                    } else {
                        // Simulate silence detection
                        control_tx.send(StreamGate::Closed).await.unwrap();
                        break;
                    }
                }
            }
        }
    });

    // Send test chunks
    for (chunk, _) in chunks {
        audio_tx.send(chunk).await.unwrap();
    }
    drop(audio_tx);

    // Verify control signals
    assert_eq!(control_rx.recv().await.unwrap(), StreamGate::Open);
    
    // Verify STT receives audio
    let mut stt_chunks_received = 0;
    while let Ok(chunk) = timeout(Duration::from_millis(100), stt_rx.recv()).await {
        if chunk.is_ok() {
            stt_chunks_received += 1;
        }
    }
    assert!(stt_chunks_received > 0, "STT should receive audio chunks");
    
    assert_eq!(control_rx.recv().await.unwrap(), StreamGate::Closed);
    
    handle.await.unwrap();
    
    println!("✅ STT channel flow test passed (received {} chunks)", stt_chunks_received);
    Ok(())
}

#[tokio::test]
async fn test_rapid_wakeword_detections() -> Result<()> {
    println!("🧪 Testing rapid wakeword detections");
    
    let config = TestConfig::default();
    let mut simulator = PipelineSimulator::new(config);

    // First wakeword detection
    simulator.detect_wakeword();
    assert_eq!(simulator.get_state(), PipelineState::WaitingForSpeech);

    // Process some speech
    simulator.process_chunk(true);
    simulator.process_chunk(true);

    // Try another wakeword detection while in speech state
    // (This should not happen in real system, but test robustness)
    let gate = simulator.detect_wakeword();
    assert_eq!(gate, StreamGate::Open);
    assert_eq!(simulator.get_state(), PipelineState::WaitingForSpeech);

    // Should still be able to end normally
    simulator.process_chunk(false);
    tokio::time::sleep(Duration::from_millis(700)).await;
    let result = simulator.process_chunk(false);
    assert_eq!(result, Some(StreamGate::Closed));
    
    println!("✅ Rapid wakeword detections test passed");
    Ok(())
} 