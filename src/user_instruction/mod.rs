use crate::{
    config::load_config,
    detection::{DetectionPipeline, PipelineConfig},
    error::{EdgeError, Result as EdgeResult},
    speech_producer::{SpeechChunk, SpeechHub},
    stt::{FireworksSTT, STTConfig},
};
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::broadcast;

/// Represents a user instruction obtained through voice
#[derive(Debug, Clone)]
pub struct UserInstruction {
    pub text: String,
    pub confidence: f32,
}

/// Configuration for user instruction detection
#[derive(Debug, Clone)]
pub struct Config {
    pub wakeword_config: PipelineConfig,
    pub stt_config: STTConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            wakeword_config: Default::default(),
            stt_config: Default::default(),
        }
    }
}

/// Handles detection of user instructions through voice
pub struct UserInstructionDetector {
    pipeline: DetectionPipeline,
    stt: Arc<FireworksSTT>,
    speech_hub: Arc<SpeechHub>,
    recent_chunks: VecDeque<SpeechChunk>, // Buffer for handling wakeword-to-STT transition
}

impl UserInstructionDetector {
    /// Create a new detector with the given configuration
    pub fn new(config: Config, speech_hub: Arc<SpeechHub>) -> EdgeResult<Self> {
        // Load API configuration
        let api_config = load_config()?;

        Ok(Self {
            pipeline: DetectionPipeline::new(config.wakeword_config)?,
            stt: Arc::new(FireworksSTT::with_config(
                api_config.fireworks_key().to_string(),
                config.stt_config,
            )),
            speech_hub,
            recent_chunks: VecDeque::with_capacity(5), // ~400ms buffer
        })
    }

    /// Process a chunk for wakeword detection
    fn check_wakeword(&mut self, chunk: &SpeechChunk) -> EdgeResult<Option<f32>> {
        // Update recent chunks buffer
        self.recent_chunks.push_back(chunk.clone());
        if self.recent_chunks.len() > 5 {
            self.recent_chunks.pop_front();
        }

        match self.pipeline.process_audio_chunk(&chunk.samples_f32) {
            Ok(detection) => {
                // Log all confidence scores for debugging
                log::debug!("🎯 Wakeword confidence: {:.3}", detection.confidence);
                // Always return the confidence score
                Ok(Some(detection.confidence))
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Get the next user instruction through voice input
    ///
    /// This function will:
    /// 1. Get a fresh speech subscription for this instruction cycle
    /// 2. Listen for wakeword and detect when it ends
    /// 3. Start STT immediately after wakeword ends
    /// 4. Return the transcribed instruction
    pub async fn get_instruction(&mut self) -> EdgeResult<UserInstruction> {
        log::debug!("🎯 Starting get_instruction - waiting for speech chunks");
        
        let mut confidence = 0.0;
        let mut wakeword_detected = false;
        let mut peak_confidence = 0.0;
        let mut chunks_since_peak = 0;
        let mut confidence_window = VecDeque::new();
        const CONFIDENCE_WINDOW_SIZE: usize = 8; // Track ~640ms of confidence scores
        const MIN_CHUNKS_AFTER_PEAK: usize = 1; // Minimum ~80ms after peak (reduced from 3)
        const CONFIDENCE_DROP_THRESHOLD: f32 = 0.10; // Smaller confidence drop needed (reduced from 0.15)

        // Timeout mechanism for stuck wakeword detection
        let mut chunks_processed = 0;
        const MAX_CHUNKS_BEFORE_RESET: usize = 100; // ~8 seconds of audio

        // Get a fresh receiver for this entire instruction cycle
        let mut speech_rx = self.speech_hub.subscribe();

        log::info!("👂 Ready for wakeword - say 'Hey Mycroft'");

        // Phase 1: Listen for wakeword and detect its end
        loop {
            match speech_rx.recv().await {
                Ok(chunk) => {
                    chunks_processed += 1;
                    log::debug!("👂 Received chunk {} for wakeword detection", chunks_processed);

                    // Reset wakeword pipeline if stuck for too long
                    if chunks_processed > MAX_CHUNKS_BEFORE_RESET {
                        log::warn!("🔄 Wakeword detection stuck - resetting pipeline");
                        self.pipeline.reset_melspec_accumulator();
                        chunks_processed = 0;
                        confidence_window.clear();
                    }

                    if let Some(conf) = self.check_wakeword(&chunk)? {
                        if !wakeword_detected && conf >= 0.09 {
                            log::info!("🎤 Wakeword detected! Monitoring for end... (confidence: {:.3})", conf);
                            wakeword_detected = true;
                            peak_confidence = conf;
                            chunks_since_peak = 0;
                        } else if wakeword_detected {
                            // Log confidence scores during wakeword detection
                            log::debug!("🎯 Monitoring wakeword - current: {:.3}, peak: {:.3}, chunks since peak: {}", 
                                conf, peak_confidence, chunks_since_peak);
                            
                            // Always increment chunks since peak when in wakeword mode
                            chunks_since_peak += 1;

                            // Update peak if we find a higher confidence
                            if conf > peak_confidence {
                                peak_confidence = conf;
                                chunks_since_peak = 0; // Reset counter when we find a new peak
                                log::debug!("🎯 New peak confidence: {:.3}", peak_confidence);
                            }
                        } else {
                            // Log all confidence scores below threshold
                            log::debug!("🎯 Below threshold - confidence: {:.3}, threshold: {:.3}", 
                                conf, 0.09);
                        }

                        confidence = conf.max(confidence);

                        // Update confidence window
                        confidence_window.push_back(conf);
                        if confidence_window.len() > CONFIDENCE_WINDOW_SIZE {
                            confidence_window.pop_front();
                        }

                        // Check for wakeword end conditions:
                        // 1. Must be at least MIN_CHUNKS_AFTER_PEAK after confidence peak
                        // 2. Current confidence must have dropped significantly from peak
                        if chunks_since_peak >= MIN_CHUNKS_AFTER_PEAK {
                            // Calculate average confidence over recent window
                            let window_avg = confidence_window.iter().sum::<f32>()
                                / confidence_window.len() as f32;

                            // Check if confidence has dropped enough from peak
                            if peak_confidence - window_avg >= CONFIDENCE_DROP_THRESHOLD {
                                log::info!("🎯 Wakeword complete - peak confidence: {:.3}", peak_confidence);
                                log::info!("🎤 Ready for speech - what can I help you with?");
                                break;
                            }
                        }
                    } else if wakeword_detected {
                        // No wakeword detected but we were in wakeword mode
                        chunks_since_peak += 1;

                        // Only end if we've accumulated enough context
                        if chunks_since_peak >= MIN_CHUNKS_AFTER_PEAK {
                            log::info!("🎯 Wakeword ended (no detection), starting STT...");
                            break;
                        }
                    }
                }
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    log::warn!("Lagged behind in speech stream, skipped {} chunks", skipped);
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => {
                    return Err(EdgeError::Audio("Speech stream closed".into()));
                }
            }
        }

        // Phase 2: Start STT with recent audio context
        self.pipeline.reset();

        // Include recent chunks in STT to capture any speech that started during wakeword detection
        let stt_receiver = self.speech_hub.subscribe();
        let recent_chunks_for_stt: Vec<SpeechChunk> = self.recent_chunks.iter().cloned().collect();

        log::info!(
            "🎤 Starting STT with {} recent chunks for context",
            recent_chunks_for_stt.len()
        );

        // Send recent chunks to STT first, then continue with live stream
        let stt_start_time = Instant::now();
        let instruction = Arc::clone(&self.stt)
            .transcribe_stream_with_context(stt_receiver, recent_chunks_for_stt)
            .await?;
        let stt_end_time = Instant::now();

        let stt_duration = stt_end_time.duration_since(stt_start_time);
        log::info!(
            "🎯 STT completed in {:.2}ms - Final instruction: '{}'",
            stt_duration.as_millis(),
            instruction
        );

        Ok(UserInstruction {
            text: instruction,
            confidence,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio_capture::AudioCaptureConfig;
    use std::env;
    use std::sync::Arc;

    // Helper function to check environment requirements
    fn check_test_requirements() -> (bool, bool) {
        let has_api_key = env::var("FIREWORKS_API_KEY").is_ok();
        let has_audio = SpeechHub::new(AudioCaptureConfig::default(), 0.5).is_ok();
        (has_api_key, has_audio)
    }

    #[tokio::test]
    async fn test_user_instruction_detector_basic() {
        // Basic test that doesn't require API or audio devices
        let config = Config::default();

        // Test that config is properly constructed
        assert_eq!(config.wakeword_config.confidence_threshold, 0.3);
        assert_eq!(
            config.stt_config.server_timeout,
            std::time::Duration::from_secs(30)
        );

        println!("✅ Basic UserInstructionDetector configuration test passed");
    }

    #[tokio::test]
    #[cfg_attr(
        not(feature = "test-api"),
        ignore = "requires API key - run with --features test-api"
    )]
    async fn test_user_instruction_detector_with_api() {
        let (has_api_key, has_audio) = check_test_requirements();

        if !has_api_key {
            panic!("This test requires FIREWORKS_API_KEY environment variable");
        }

        if !has_audio {
            panic!("This test requires an available audio input device");
        }

        // Create speech hub
        let speech_hub = Arc::new(SpeechHub::new(AudioCaptureConfig::default(), 0.5).unwrap());

        // Create detector
        let config = Config::default();
        let detector = UserInstructionDetector::new(config, speech_hub);
        assert!(detector.is_ok());
        println!("✅ UserInstructionDetector created successfully");
    }

    #[tokio::test]
    #[cfg_attr(
        not(feature = "test-audio"),
        ignore = "requires audio device - run with --features test-audio"
    )]
    async fn test_user_instruction_detector_audio_only() {
        let (_, has_audio) = check_test_requirements();

        if !has_audio {
            panic!("This test requires an available audio input device");
        }

        // Test just the audio components without API dependency
        let speech_hub_result = SpeechHub::new(AudioCaptureConfig::default(), 0.5);
        assert!(speech_hub_result.is_ok());
        println!("✅ Audio device is available and working");
    }

    // This test will always run and report what's available
    #[tokio::test]
    async fn test_environment_capabilities() {
        let (has_api_key, has_audio) = check_test_requirements();

        println!("🔍 Environment Capabilities:");
        println!(
            "  - API Key (FIREWORKS_API_KEY): {}",
            if has_api_key {
                "✅ Available"
            } else {
                "❌ Missing"
            }
        );
        println!(
            "  - Audio Device: {}",
            if has_audio {
                "✅ Available"
            } else {
                "❌ Missing"
            }
        );

        if !has_api_key && !has_audio {
            println!("💡 To run full tests:");
            println!("  - Set FIREWORKS_API_KEY environment variable");
            println!("  - Ensure audio input device is available");
            println!("  - Run: cargo test --features test-api,test-audio");
        } else if !has_api_key {
            println!("💡 To run API tests: cargo test --features test-api");
        } else if !has_audio {
            println!("💡 To run audio tests: cargo test --features test-audio");
        }

        // This test always passes but provides useful information
        assert!(true, "Environment check completed");
    }
}
