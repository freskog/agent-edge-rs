//! # STT State Machine Tests
//!
//! Tests for the STT state machine behavior, timing logic, and speech detection
//! without requiring actual audio file processing or API calls.

use agent_edge_rs::error::Result;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Test configuration matching main.rs pipeline
struct TestConfig {
    first_speech_timeout_ms: u128,
    silence_timeout_ms: u128,
    recent_chunks_size: usize,
    _chunk_size: usize,
}

impl Default for TestConfig {
    fn default() -> Self {
        Self {
            first_speech_timeout_ms: 8000, // 8 seconds
            silence_timeout_ms: 650,       // 0.65 seconds
            recent_chunks_size: 5,
            _chunk_size: 1280, // 80ms at 16kHz
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum PipelineState {
    WaitingForWakeword,
    WaitingForSpeech,
}

/// Simulate the STT state machine with different speech patterns
struct STTStateMachine {
    config: TestConfig,
    state: PipelineState,
    recent_chunks: VecDeque<bool>,
    speech_timeout: Instant,
    silence_start: Option<Instant>,
    user_speaking_immediately: bool,
    stt_opened: bool,
    stt_closed: bool,
    stt_close_reason: String,
}

impl STTStateMachine {
    fn new() -> Self {
        Self {
            config: TestConfig::default(),
            state: PipelineState::WaitingForWakeword,
            recent_chunks: VecDeque::new(),
            speech_timeout: Instant::now(),
            silence_start: None,
            user_speaking_immediately: false,
            stt_opened: false,
            stt_closed: false,
            stt_close_reason: String::new(),
        }
    }

    fn detect_wakeword(&mut self) {
        // Check if user is speaking immediately after wakeword
        let recent_speech_count = self
            .recent_chunks
            .iter()
            .filter(|&&is_speech| is_speech)
            .count();
        self.user_speaking_immediately = recent_speech_count >= 3;

        self.state = PipelineState::WaitingForSpeech;
        self.speech_timeout = Instant::now();
        self.silence_start = None;
        self.stt_opened = true;

        println!(
            "🎉 Wakeword detected! STT opened. Immediate speech: {}",
            self.user_speaking_immediately
        );
    }

    fn process_chunk(&mut self, has_speech: bool) -> bool {
        // Track recent speech activity
        self.recent_chunks.push_back(has_speech);
        if self.recent_chunks.len() > self.config.recent_chunks_size {
            self.recent_chunks.pop_front();
        }

        match self.state {
            PipelineState::WaitingForWakeword => {
                // Would run wakeword detection in real implementation
                false
            }
            PipelineState::WaitingForSpeech => {
                if has_speech {
                    // Speech detected - reset silence timer
                    self.silence_start = None;
                    false
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

                    // Check for first speech timeout
                    let speech_elapsed = self.speech_timeout.elapsed().as_millis();
                    let first_speech_timeout = !self.user_speaking_immediately
                        && speech_elapsed > self.config.first_speech_timeout_ms;

                    if should_end_stt || first_speech_timeout {
                        self.state = PipelineState::WaitingForWakeword;
                        self.stt_closed = true;
                        self.stt_close_reason = if should_end_stt {
                            "silence".to_string()
                        } else {
                            "timeout".to_string()
                        };

                        println!("🔇 STT closed due to {}", self.stt_close_reason);
                        return true;
                    }
                    false
                }
            }
        }
    }
}

/// Create test speech patterns for different scenarios
fn create_speech_pattern(pattern_type: &str) -> Vec<bool> {
    match pattern_type {
        "immediate" => {
            // Pattern: wakeword + immediate speech + extended silence (should close due to silence)
            vec![
                true, true, true, true, true, true, true, false, false, false, false, false, false,
                false, false, false, false, false,
            ]
        }
        "delayed" => {
            // Pattern: wakeword + pause + speech + extended silence (should close due to silence)
            vec![
                true, false, false, false, true, true, true, true, false, false, false, false,
                false, false, false, false, false, false,
            ]
        }
        "hesitation" => {
            // Pattern: wakeword + hesitation + speech + extended silence (should close due to silence)
            vec![
                true, true, false, true, false, true, true, true, false, false, false, false,
                false, false, false, false, false, false,
            ]
        }
        "timeout" => {
            // Pattern: wakeword + long silence (should timeout)
            vec![
                true, false, false, false, false, false, false, false, false, false, false, false,
                false, false, false, false, false, false,
            ]
        }
        _ => vec![false; 18],
    }
}

#[tokio::test]
async fn test_immediate_speech_state_machine() -> Result<()> {
    println!("🧪 Testing immediate speech state machine");

    let mut stt = STTStateMachine::new();
    let pattern = create_speech_pattern("immediate");

    // Build recent speech history (immediate speech)
    for &has_speech in &pattern[..5] {
        stt.process_chunk(has_speech);
    }

    // Detect wakeword
    stt.detect_wakeword();
    assert!(stt.stt_opened);
    assert!(stt.user_speaking_immediately);

    // Process speech chunks
    for &has_speech in &pattern[5..7] {
        stt.process_chunk(has_speech);
    }

    // Process silence chunks with time simulation
    for &has_speech in &pattern[7..] {
        if !has_speech && stt.silence_start.is_none() {
            // Start silence timer
            stt.process_chunk(has_speech);
            // Simulate silence duration exceeding threshold
            stt.silence_start = Some(Instant::now() - Duration::from_millis(700));
            // 700ms ago
        }
        if stt.process_chunk(has_speech) {
            break; // STT closed
        }
    }

    assert!(stt.stt_closed);
    assert_eq!(stt.stt_close_reason, "silence");

    println!("✅ Immediate speech state machine test passed");
    Ok(())
}

#[tokio::test]
async fn test_delayed_speech_state_machine() -> Result<()> {
    println!("🧪 Testing delayed speech state machine");

    let mut stt = STTStateMachine::new();
    let pattern = create_speech_pattern("delayed");

    // Build recent speech history (delayed speech)
    for &has_speech in &pattern[..5] {
        stt.process_chunk(has_speech);
    }

    // Detect wakeword
    stt.detect_wakeword();
    assert!(stt.stt_opened);
    assert!(!stt.user_speaking_immediately); // Should detect delayed speech

    // Process speech chunks
    for &has_speech in &pattern[5..8] {
        stt.process_chunk(has_speech);
    }

    // Process silence chunks with time simulation
    for &has_speech in &pattern[8..] {
        if !has_speech && stt.silence_start.is_none() {
            // Start silence timer
            stt.process_chunk(has_speech);
            // Simulate silence duration exceeding threshold
            stt.silence_start = Some(Instant::now() - Duration::from_millis(700));
            // 700ms ago
        }
        if stt.process_chunk(has_speech) {
            break; // STT closed
        }
    }

    assert!(stt.stt_closed);
    assert_eq!(stt.stt_close_reason, "silence");

    println!("✅ Delayed speech state machine test passed");
    Ok(())
}

#[tokio::test]
async fn test_hesitation_speech_state_machine() -> Result<()> {
    println!("🧪 Testing hesitation speech state machine");

    let mut stt = STTStateMachine::new();
    let pattern = create_speech_pattern("hesitation");

    // Build recent speech history (hesitation pattern)
    for &has_speech in &pattern[..5] {
        stt.process_chunk(has_speech);
    }

    // Detect wakeword
    stt.detect_wakeword();
    assert!(stt.stt_opened);

    // Process speech chunks
    for &has_speech in &pattern[5..8] {
        stt.process_chunk(has_speech);
    }

    // Process silence chunks with time simulation
    for &has_speech in &pattern[8..] {
        if !has_speech && stt.silence_start.is_none() {
            // Start silence timer
            stt.process_chunk(has_speech);
            // Simulate silence duration exceeding threshold
            stt.silence_start = Some(Instant::now() - Duration::from_millis(700));
            // 700ms ago
        }
        if stt.process_chunk(has_speech) {
            break; // STT closed
        }
    }

    assert!(stt.stt_closed);
    assert_eq!(stt.stt_close_reason, "silence");

    println!("✅ Hesitation speech state machine test passed");
    Ok(())
}

#[tokio::test]
async fn test_timeout_state_machine() -> Result<()> {
    println!("🧪 Testing timeout state machine");

    let mut stt = STTStateMachine::new();
    let pattern = create_speech_pattern("timeout");

    // Build recent speech history (no immediate speech)
    for &has_speech in &pattern[..5] {
        stt.process_chunk(has_speech);
    }

    // Detect wakeword
    stt.detect_wakeword();
    assert!(stt.stt_opened);
    assert!(!stt.user_speaking_immediately);

    // Simulate timeout by advancing time
    stt.speech_timeout = Instant::now() - Duration::from_millis(9000); // 9 seconds ago

    // Process silence - should timeout
    let closed = stt.process_chunk(false);
    assert!(closed);
    assert!(stt.stt_closed);
    assert_eq!(stt.stt_close_reason, "timeout");

    println!("✅ Timeout state machine test passed");
    Ok(())
}

#[tokio::test]
async fn test_silence_gap_timing() -> Result<()> {
    println!("🧪 Testing silence gap timing");

    let mut stt = STTStateMachine::new();

    // Setup for immediate speech
    for _ in 0..3 {
        stt.process_chunk(true);
    }

    stt.detect_wakeword();
    assert!(stt.stt_opened);

    // Process speech, then silence
    stt.process_chunk(true); // Speech
    stt.process_chunk(false); // Start silence

    // Simulate silence duration just under threshold
    stt.silence_start = Some(Instant::now() - Duration::from_millis(600)); // 600ms ago
    let closed = stt.process_chunk(false);
    assert!(!closed); // Should not close yet

    // Simulate silence duration over threshold
    stt.silence_start = Some(Instant::now() - Duration::from_millis(700)); // 700ms ago
    let closed = stt.process_chunk(false);
    assert!(closed); // Should close now

    assert!(stt.stt_closed);
    assert_eq!(stt.stt_close_reason, "silence");

    println!("✅ Silence gap timing test passed");
    Ok(())
}
