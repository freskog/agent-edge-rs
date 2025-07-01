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
            Ok(detection) if detection.detected => Ok(Some(detection.confidence)),
            Ok(_) => Ok(None),
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

        // Phase 1: Listen for wakeword and detect its end
        loop {
            match speech_rx.recv().await {
                Ok(chunk) => {
                    chunks_processed += 1;

                    // Reset wakeword pipeline if stuck for too long
                    if chunks_processed > MAX_CHUNKS_BEFORE_RESET {
                        log::warn!("🔄 Wakeword detection stuck - resetting pipeline");
                        self.pipeline.reset_melspec_accumulator();
                        chunks_processed = 0;
                        confidence_window.clear();
                    }

                    if let Some(conf) = self.check_wakeword(&chunk)? {
                        if !wakeword_detected {
                            log::info!("🎤 Wakeword detected! Monitoring for end...");
                            wakeword_detected = true;
                            peak_confidence = conf;
                            chunks_since_peak = 0;
                        } else {
                            // Always increment chunks since peak when in wakeword mode
                            chunks_since_peak += 1;

                            // Update peak if we find a higher confidence
                            if conf > peak_confidence {
                                peak_confidence = conf;
                                chunks_since_peak = 0; // Reset counter when we find a new peak
                            }
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
                        // 3. Recent confidence window should show consistent decline
                        if chunks_since_peak >= MIN_CHUNKS_AFTER_PEAK {
                            let current_conf = conf;
                            let conf_drop = peak_confidence - current_conf;

                            // Calculate if confidence is consistently declining
                            let mut is_declining = true;
                            if confidence_window.len() >= 3 {
                                let window: Vec<f32> = confidence_window.iter().copied().collect();
                                for i in 1..window.len() {
                                    if window[i] > window[i - 1] {
                                        is_declining = false;
                                        break;
                                    }
                                }
                            }

                            if conf_drop > CONFIDENCE_DROP_THRESHOLD && is_declining {
                                log::info!(
                                    "🎯 Wakeword ended (peak: {:.3}, current: {:.3}, drop: {:.3}), starting STT...",
                                    peak_confidence,
                                    current_conf,
                                    conf_drop
                                );
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

    #[tokio::test]
    async fn test_user_instruction_detector() {
        // Skip test if no API key
        if env::var("FIREWORKS_API_KEY").is_err() {
            println!("⏭️  Skipping test - FIREWORKS_API_KEY not found");
            return;
        }

        // Create speech hub
        let speech_hub = Arc::new(SpeechHub::new(AudioCaptureConfig::default(), 0.5).unwrap());

        // Create detector
        let config = Config::default();
        let detector = UserInstructionDetector::new(config, speech_hub);
        assert!(detector.is_ok());
    }
}
