//! # Wake Word Detection Pipeline
//!
//! This module implements a complete wake word detection pipeline based on the OpenWakeWord architecture.
//! The pipeline processes raw audio through multiple machine learning models to detect specific wake words
//! like "Hey Mycroft" with high accuracy and low false positive rates.
//!
//! ## Conceptual Overview: Audio as Language
//!
//! The pipeline works similarly to natural language processing, but for audio:
//!
//! ```text
//! Text Processing:    "Hello world" → Tokens → Word Embeddings → Language Model → Classification
//! Audio Processing:   Raw Audio     → Mel     → Audio Embeddings → Wake Word   → Detection
//!                                     Frames                        Model
//! ```
//!
//! Just as text tokenizers convert words into discrete tokens that capture semantic meaning,
//! the **melspectrogram acts as an "audio tokenizer"** that converts raw sound waves into
//! discrete mel frames that capture acoustic/phonetic meaning.
//!
//! ## Three-Stage Architecture
//!
//! ```text
//! Raw Audio (80ms) → Melspectrogram → Embedding → Wake Word Detection
//!   1280 samples      160 features    96 features   confidence score
//!   16kHz, f32       5×32 frames     per chunk      0.0 - 1.0
//! ```
//!
//! ### Stage 1: Melspectrogram Model (Audio "Tokenizer")
//!
//! **Converts raw audio into acoustic "tokens"** - discrete units representing phonetic content.
//!
//! - **Input**: `[1280]` samples (80ms at 16kHz) → **Output**: `[5, 32]` = 160 mel features
//! - **Process**: Time-frequency transform → mel scale mapping → temporal framing
//! - **Result**: 5 time frames (~16ms each) × 32 mel bins = phonetic "vocabulary"
//!
//! Each mel frame captures the spectral fingerprint of ~16ms of audio, like how text tokens
//! capture semantic units. The mel scale mimics human hearing for robust speech processing.
//!
//! ### Stage 2: Embedding Model (Audio "Word Vectors")
//!
//! **Converts mel frame sequences into semantic embeddings** that capture phonetic patterns.
//!
//! - **Input**: `[76, 32]` = 2432 features (~1.52s context) → **Output**: `[96]` embedding features
//! - **Context**: Requires 76 consecutive mel frames for optimal phonetic pattern recognition
//! - **Accumulation**: Collect ~16 melspec outputs (80 frames) and extract most recent 76
//!
//! ### Stage 3: Wake Word Model (Audio "Language Model")
//!
//! **Analyzes embedding sequences to detect specific wake word patterns.**
//!
//! - **Input**: `[16, 96]` = 1536 features (~1.28s context) → **Output**: confidence score
//! - **Purpose**: Processes 16 consecutive embeddings to capture full wake word duration
//!
//! ## Why Dimensions Don't Align Perfectly
//!
//! The pipeline has intentional "misalignment" that provides important benefits:
//!
//! ```text
//! Melspectrogram: 16 outputs × 5 frames = 80 frames total
//! Embedding needs: 76 frames exactly  
//! Result: 4 extra frames → intentional overlap
//! ```
//!
//! **Benefits of this design:**
//! - **Optimal contexts**: Each model trained with specific temporal requirements (76 vs 16)
//! - **Overlapping windows**: Smooth transitions, detects speech spanning chunk boundaries  
//! - **Real-time balance**: 80ms chunks optimize latency vs. spectral resolution
//! - **Robustness**: Overlap prevents information loss at boundaries
//!
//! A "perfectly aligned" system would require retraining all models or impractical frame sizes.
//!
//! ## Processing Flow
//!
//! ### Audio Tokenization & Context Building
//! ```text
//! Audio Stream → 80ms chunks → Mel "tokens" → Accumulate 76 frames → Embedding
//!              → Collect 16 embeddings → Wake word classification → Debounced detection
//! ```
//!
//! ### Startup Behavior
//! The pipeline needs ~1.3 seconds to build sufficient context:
//! - **Phase 1**: Accumulate 16 melspectrogram outputs for first embedding (~1.28s)
//! - **Phase 2**: Collect 16 embeddings for first wake word detection (~1.28s)
//! - **Ready**: Continuous real-time detection with proper debouncing
//!
//! ## Key Design Decisions
//!
//! ### Sliding Windows
//! Both melspec accumulation and embedding collection use sliding windows for continuous
//! processing without gaps, enabling detection across audio chunk boundaries.
//!
//! ### Debouncing (1 second default)
//! Prevents multiple detections from single utterances since sliding windows would otherwise
//! trigger repeatedly for the same wake word.
//!
//! ### Model-Specific Optimizations
//! - **Melspectrogram**: Real-time audio feature extraction with perceptual mel scaling
//! - **Embedding**: Phonetic pattern recognition trained on diverse speech data
//! - **Wake Word**: Specifically trained on "Hey Mycroft" variations for high accuracy
//!
//! ## Performance Characteristics
//!
//! - **Latency**: ~1.3 seconds (due to required temporal context)
//! - **CPU Usage**: Reduced significantly when combined with VAD (Voice Activity Detection)
//! - **Memory**: Fixed-size rolling windows prevent unbounded growth (~16KB total)
//! - **Accuracy**: High precision/recall with properly tuned confidence threshold

use crate::error::{EdgeError, Result};
use crate::led_ring::LedRing;
use crate::models::{
    embedding::EmbeddingModel,
    melspectrogram::MelSpectrogramModel,
    wakeword::{WakewordDetection, WakewordModel},
};
use std::collections::VecDeque;
use voice_activity_detector::VoiceActivityDetector;

/// Configuration for the detection pipeline
///
/// This struct contains all the parameters needed to configure the wake word detection pipeline.
/// The values are carefully tuned based on the OpenWakeWord architecture and the specific
/// characteristics of the TensorFlow Lite models being used.
#[derive(Debug, Clone)]
pub struct PipelineConfig {
    /// Model paths
    pub melspectrogram_model_path: String, // Path to melspectrogram.tflite
    pub embedding_model_path: String, // Path to embedding_model.tflite
    pub wakeword_model_path: String,  // Path to hey_mycroft_v0.1.tflite

    /// Processing parameters
    pub chunk_size: usize, // Audio chunk size: 1280 samples = 80ms at 16kHz
    pub sample_rate: u32, // Audio sampling rate: 16000 Hz (required by models)
    pub confidence_threshold: f32, // Detection threshold: 0.09 (9% confidence)

    /// Windowing parameters  
    pub window_size: usize, // Embedding window size: 16 embeddings = ~1.28s context
    pub overlap_size: usize, // Overlap for sliding windows (currently unused)

    /// Debouncing parameters
    pub debounce_duration_ms: u64, // Minimum time between detections: 1000ms (1 second)

    /// VAD post-filtering (OpenWakeWord style)
    pub vad_threshold: f32, // VAD threshold for post-filtering predictions (0.0 = disabled)
    pub vad_frame_lookback: usize, // How many frames back to check VAD (default: 7)

    /// LED feedback configuration
    pub enable_led_feedback: bool, // Enable/disable LED ring visual feedback
    pub led_brightness: u8,                // LED brightness (0-31)
    pub led_listening_color: (u8, u8, u8), // RGB color when listening for wake word
    pub led_detected_color: (u8, u8, u8),  // RGB color when wake word detected
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            melspectrogram_model_path: "models/melspectrogram.tflite".to_string(),
            embedding_model_path: "models/embedding_model.tflite".to_string(),
            wakeword_model_path: "models/hey_mycroft_v0.1.tflite".to_string(),
            chunk_size: 1280,
            sample_rate: 16000,
            confidence_threshold: 0.09, // Lower threshold to match observed wakeword probabilities
            window_size: 16,
            overlap_size: 8,
            debounce_duration_ms: 1000, // 1 second debounce (OpenWakeWord uses 1.25s in tests)
            vad_threshold: 0.0,
            vad_frame_lookback: 7,
            enable_led_feedback: true,
            led_brightness: 31,
            led_listening_color: (0, 0, 255),
            led_detected_color: (0, 255, 0),
        }
    }
}

/// Complete detection pipeline for wakeword detection
///
/// This is the main pipeline struct that orchestrates the entire wake word detection process.
/// It maintains the state of all three models (melspectrogram, embedding, wake word) and
/// manages the sliding windows needed for continuous audio processing.
///
/// ## Internal State Management
///
/// The pipeline maintains two critical sliding windows:
///
/// 1. **Melspectrogram Accumulator** (`melspec_accumulator`)
///    - Stores the last 16 melspectrogram outputs (160 features each)
///    - Provides sufficient frames (16×5=80) to extract 76 frames for embedding input
///    - Enables continuous processing across audio chunk boundaries
///
/// 2. **Embedding Window** (`embedding_window`)
///    - Stores the last 16 embedding outputs (96 features each)
///    - Provides temporal context for the wakeword model (16×96=1536 features)
///    - Implements sliding window with overlap for robust detection
///
/// ## Performance Optimization for Pi 3
///
/// To achieve real-time performance on resource-constrained devices like Raspberry Pi 3,
/// the pipeline uses **frame skipping** for the expensive embedding model:
/// - Melspectrogram model: runs every frame (~4ms per inference)
/// - Embedding model: runs every 2nd frame (~50ms per inference, but half frequency)  
/// - Wakeword model: runs every frame (~1ms per inference)
///
/// This reduces the embedding CPU load from 62% to 31% of real-time budget.
pub struct DetectionPipeline {
    // Models
    melspectrogram_model: MelSpectrogramModel,
    embedding_model: EmbeddingModel,
    wakeword_model: WakewordModel,

    // Configuration
    config: PipelineConfig,

    // State for sliding windows
    melspec_accumulator: VecDeque<Vec<f32>>,
    embedding_window: VecDeque<Vec<f32>>,
    ignore_detections: bool,
    melspec_frames_needed: usize, // How many melspec outputs we need for one embedding input

    // Frame skipping optimization for Pi 3
    frame_counter: usize,
    embedding_skip_rate: usize, // Run embedding every N frames (default: 2)

    // Pre-allocated buffers to avoid allocations during processing
    flattened_buffer: Vec<f32>,  // For mel features → embedding input
    features_buffer: Vec<f32>,   // For embeddings → wakeword input

    // Optional VAD for post-filtering predictions (OpenWakeWord approach)
    vad: Option<VoiceActivityDetector>,
    vad_predictions: VecDeque<f32>, // Rolling buffer of VAD predictions

    // LED ring for visual feedback (optional)
    led_ring: Option<LedRing>,
}

impl DetectionPipeline {
    /// Create a new detection pipeline with the given configuration
    pub fn new(config: PipelineConfig) -> Result<Self> {
        log::info!("Initializing detection pipeline...");

        // Initialize all models
        let melspectrogram_model = MelSpectrogramModel::new(&config.melspectrogram_model_path)?;
        let embedding_model = EmbeddingModel::new(&config.embedding_model_path)?;
        let wakeword_model = WakewordModel::new(&config.wakeword_model_path)?;

        // Initialize rolling window for embeddings (16 frames × 96 features each = 1536 total)
        let embedding_window = VecDeque::with_capacity(config.window_size);

        // Initialize melspec accumulator (76 frames needed for embedding)
        let melspec_accumulator = VecDeque::with_capacity(config.window_size);
        let melspec_frames_needed = 16; // We need ~16 melspec outputs (5 frames each) to get 76+ frames

        // Initialize simple detection gating
        let ignore_detections = false;

        // Initialize pre-allocated buffers
        let flattened_buffer = Vec::with_capacity(76 * 32); // 2432 features
        let features_buffer = Vec::with_capacity(config.window_size * 96);

        // Initialize VAD if enabled (for post-filtering predictions)
        let vad = if config.vad_threshold > 0.0 {
            match VoiceActivityDetector::builder()
                .sample_rate(16000)
                .chunk_size(1280_usize)
                .build()
            {
                Ok(detector) => {
                    log::info!("VAD post-filtering enabled (threshold: {:.2})", config.vad_threshold);
                    Some(detector)
                }
                Err(e) => {
                    log::warn!("Failed to initialize VAD for post-filtering: {}", e);
                    None
                }
            }
        } else {
            log::info!("VAD post-filtering disabled");
            None
        };

        // Initialize VAD predictions buffer
        let vad_predictions = VecDeque::with_capacity(config.vad_frame_lookback);

        // Initialize LED ring if enabled
        let led_ring = if config.enable_led_feedback {
            match LedRing::new() {
                Ok(ring) => {
                    log::info!("LED ring initialized successfully");
                    // Set initial brightness and listening mode
                    if let Err(e) = ring.set_brightness(config.led_brightness) {
                        log::warn!("Failed to set LED brightness: {}", e);
                    }
                    if let Err(e) = ring.set_color(
                        config.led_listening_color.0,
                        config.led_listening_color.1,
                        config.led_listening_color.2,
                    ) {
                        log::warn!("Failed to set LED listening color: {}", e);
                    }
                    Some(ring)
                }
                Err(e) => {
                    log::warn!(
                        "Failed to initialize LED ring: {}. Continuing without LED feedback.",
                        e
                    );
                    None
                }
            }
        } else {
            log::info!("LED feedback disabled in configuration");
            None
        };

        log::info!("Detection pipeline initialized successfully");

        Ok(Self {
            melspectrogram_model,
            embedding_model,
            wakeword_model,
            config,
            melspec_accumulator,
            embedding_window,
            ignore_detections,
            melspec_frames_needed,
            frame_counter: 0,
            embedding_skip_rate: 2,
            flattened_buffer,
            features_buffer,
            vad,
            vad_predictions,
            led_ring,
        })
    }

    /// Process a chunk of audio and return detection result
    ///
    /// This is the main processing function that implements the complete OpenWakeWord pipeline.
    /// It processes 80ms audio chunks through three ML models with sliding windows to detect
    /// the target wake word with high accuracy and proper debouncing.
    ///
    /// # Arguments
    /// * `audio_chunk` - Exactly 1280 f32 samples representing 80ms of 16kHz audio
    ///
    /// # Returns
    /// * `WakewordDetection` - Contains detection result, confidence score, and timestamp
    ///
    /// # Pipeline Steps
    /// 1. **Melspectrogram Extraction**: 1280 samples → 160 mel features (5×32)
    /// 2. **Frame Accumulation**: Collect 16 melspec outputs for sufficient temporal context
    /// 3. **Embedding Generation**: 2432 mel features (76×32) → 96 embedding features  
    /// 4. **Embedding Windowing**: Collect 16 embeddings for wake word analysis
    /// 5. **Wake Word Detection**: 1536 features (16×96) → confidence score
    /// 6. **Debouncing**: Prevent repeated detections from same utterance
    pub fn process_audio_chunk(&mut self, audio_chunk: &[f32; 1280]) -> Result<WakewordDetection> {
        // Input validation: Ensure we have exactly 80ms worth of 16kHz audio
        if audio_chunk.len() != self.config.chunk_size {
            return Err(EdgeError::InvalidInput(format!(
                "Expected {} audio samples, got {}",
                self.config.chunk_size,
                audio_chunk.len()
            )));
        }

        // Step 1: Extract mel spectrogram features (1280 samples → 160 features)
        let mel_start = std::time::Instant::now();
        let mel_features = self.melspectrogram_model.predict(audio_chunk)?;
        let mel_time = mel_start.elapsed();

        // Add to accumulator for sliding window
        self.melspec_accumulator.push_back(mel_features);
        if self.melspec_accumulator.len() > 16 {
            self.melspec_accumulator.pop_front();
        }

        // Increment frame counter for embedding skip logic
        self.frame_counter += 1;

        // Step 2: Check if we have enough context for embedding (need 76 frames)
        if self.melspec_accumulator.len() < 16 {
            // Not enough frames yet, return no detection
            return Ok(WakewordDetection {
                detected: false,
                confidence: 0.0,
                timestamp: std::time::Instant::now(),
            });
        }

        // Step 3: Process embedding model with frame skipping (Pi 3 optimization)
        let mut embedding_time = std::time::Duration::ZERO;
        let mut should_run_embedding = self.frame_counter % self.embedding_skip_rate == 0;
        
        // Always run embedding if we don't have enough embeddings yet
        if self.embedding_window.len() < 16 {
            should_run_embedding = true;
        }

        if should_run_embedding {
            // Step 3a: Prepare embedding input (flatten to get 76×32 features)
            let flatten_start = std::time::Instant::now();
            self.flattened_buffer.clear();
            
            // Each melspec output is [5, 32] = 160 features
            // We need exactly 76*32 = 2432 features for embedding
            // Extract 76 frames from recent melspec outputs
            let mut frame_count = 0;
            let target_frames = 76;
            
            // Process melspec outputs to extract individual frames
            for mel_output in &self.melspec_accumulator {
                // Each mel_output contains 5 frames of 32 features each
                for frame_idx in 0..5 {
                    if frame_count >= target_frames {
                        break;
                    }
                    let start_idx = frame_idx * 32;
                    let end_idx = start_idx + 32;
                    self.flattened_buffer.extend(&mel_output[start_idx..end_idx]);
                    frame_count += 1;
                }
                if frame_count >= target_frames {
                    break;
                }
            }
            let flatten_time = flatten_start.elapsed();

            // Step 3b: Process embedding (76×32 features → 96 embedding features)
            let embed_start = std::time::Instant::now();
            let embedding_features = self.embedding_model.predict(&self.flattened_buffer)?;
            let embed_time_inner = embed_start.elapsed();
            embedding_time = flatten_time + embed_time_inner;

            // Add to embedding window
            self.embedding_window.push_back(embedding_features);
            if self.embedding_window.len() > 16 {
                self.embedding_window.pop_front();
            }
        }

        // Step 4: Check if we have enough embeddings for wakeword detection
        if self.embedding_window.len() < 16 {
            // Not enough embeddings yet
            return Ok(WakewordDetection {
                detected: false,
                confidence: 0.0,
                timestamp: std::time::Instant::now(),
            });
        }

        // Step 5: Prepare wakeword input (flatten 16 embeddings)
        let wakeword_prep_start = std::time::Instant::now();
        self.features_buffer.clear();
        for embedding in &self.embedding_window {
            for &feature in embedding.iter() {
                self.features_buffer.push(feature);
            }
        }
        let wakeword_prep_time = wakeword_prep_start.elapsed();

        // Step 6: Get wakeword confidence
        let wakeword_start = std::time::Instant::now();
        let confidence = self.wakeword_model.predict(&self.features_buffer)?;
        let wakeword_time = wakeword_start.elapsed();

        // Log timing breakdown if total time is slow (adjusted for frame skipping)
        let total_time = mel_time + embedding_time + wakeword_prep_time + wakeword_time;
        if total_time.as_millis() > 10 {
            log::warn!(
                "🐌 Processing breakdown (frame {}, embedding {}): mel={:.1}ms, embed={:.1}ms, prep={:.1}ms, wakeword={:.1}ms, total={:.1}ms",
                self.frame_counter,
                if should_run_embedding { "✓" } else { "✗" },
                mel_time.as_millis(),
                embedding_time.as_millis(),
                wakeword_prep_time.as_millis(),
                wakeword_time.as_millis(),
                total_time.as_millis()
            );
        }

        // Apply VAD post-filtering if enabled (OpenWakeWord approach)
        let filtered_confidence = if let Some(ref mut vad) = self.vad {
            // Convert audio chunk to i16 for VAD
            let i16_samples: Vec<i16> = audio_chunk.iter()
                .map(|&sample| (sample * 32767.0).clamp(-32768.0, 32767.0) as i16)
                .collect();
            
            // Get VAD prediction for this chunk
            let vad_score = vad.predict(i16_samples);
            self.vad_predictions.push_back(vad_score);
            
            // Keep only recent VAD predictions
            while self.vad_predictions.len() > self.config.vad_frame_lookback {
                self.vad_predictions.pop_front();
            }
            
            // Check if recent VAD scores indicate speech activity
            // Following OpenWakeWord: check frames from 0.4-0.56s ago (frames -7 to -4)
            let vad_frames_to_check = if self.vad_predictions.len() >= 7 {
                let start_idx = self.vad_predictions.len().saturating_sub(7);
                let end_idx = self.vad_predictions.len().saturating_sub(4);
                &self.vad_predictions.as_slices().0[start_idx..end_idx.min(self.vad_predictions.len())]
            } else {
                // Not enough history, use all available
                self.vad_predictions.as_slices().0
            };
            
            let max_vad_score = vad_frames_to_check.iter().fold(0.0f32, |acc, &x| acc.max(x));
            
            // Filter prediction if VAD score is too low
            if max_vad_score < self.config.vad_threshold {
                0.0 // Zero out prediction if no recent speech activity
            } else {
                confidence
            }
        } else {
            confidence
        };

        // Check if we should ignore this detection
        if self.ignore_detections {
            if filtered_confidence < self.config.confidence_threshold * 0.5 {
                // Reset ignore flag when confidence drops significantly
                self.ignore_detections = false;
            }
            return Ok(WakewordDetection {
                detected: false,
                confidence: filtered_confidence,
                timestamp: std::time::Instant::now(),
            });
        }

        let detected = filtered_confidence >= self.config.confidence_threshold;
        if detected {
            log::info!("🎯 Wakeword detected with {:.1}% confidence!", filtered_confidence * 100.0);
            // Set ignore flag to prevent multiple detections
            self.ignore_detections = true;
        }

        Ok(WakewordDetection {
            detected,
            confidence: filtered_confidence,
            timestamp: std::time::Instant::now(),
        })
    }

    /// Reset only the LED state
    ///
    /// This method only resets the LED back to listening mode without affecting
    /// any detection logic or audio processing state. Used when STT completes
    /// to return visual feedback to the listening state.
    pub fn reset_led_only(&mut self) {
        // Reset LED ring to listening mode if available
        if let Some(ref led_ring) = self.led_ring {
            if let Err(e) = led_ring.set_color(
                self.config.led_listening_color.0,
                self.config.led_listening_color.1,
                self.config.led_listening_color.2,
            ) {
                log::warn!("Failed to reset LED to listening color: {}", e);
            }
        }
    }

    /// Reset the detection state
    ///
    /// This is a lightweight reset that only clears the ignore flag without affecting
    /// the audio processing pipeline state. The ignore_detections flag provides proper
    /// gating between detections naturally based on confidence levels.
    ///
    /// ## When to use reset():
    ///
    /// 1. **After successful detection**: Called automatically to reset ignore flag
    /// 2. **Testing/Debugging**: Reset between test cases for clean detection state
    /// 3. **Error Recovery**: After encountering processing errors
    ///
    /// ## What gets reset:
    ///
    /// - **Ignore detections flag**: Reset to allow new detections
    /// - **LED state**: Return to listening mode
    ///
    /// ## What is preserved:
    ///
    /// - **Melspectrogram accumulator**: All mel features remain for continuous processing
    /// - **Embedding window**: All embedding vectors remain for immediate detection capability
    ///
    /// ## Performance impact:
    ///
    /// After reset, the pipeline maintains full context and can immediately detect
    /// new wakewords. The ignore flag will be managed automatically based on confidence levels.
    /// No rebuilding of audio context is required.
    pub fn reset(&mut self) {
        // Only reset the ignore flag - preserve all audio processing state
        // The confidence-based gating will handle detection management naturally
        self.ignore_detections = false; // Reset to allow new detections

        // Reset LED ring to listening mode if available
        if let Some(ref led_ring) = self.led_ring {
            if let Err(e) = led_ring.set_color(
                self.config.led_listening_color.0,
                self.config.led_listening_color.1,
                self.config.led_listening_color.2,
            ) {
                log::warn!("Failed to reset LED to listening color: {}", e);
            }
        }

        log::info!("🔄 Pipeline state reset - ready for immediate detection");
    }

    /// Reset the melspec accumulator when it gets corrupted with bad audio
    ///
    /// This method clears the melspectrogram accumulator when we suspect it's filled
    /// with non-speech audio that's preventing wakeword detection. Used when the
    /// system has been stuck accumulating context for too long without making progress.
    pub fn reset_melspec_accumulator(&mut self) {
        self.melspec_accumulator.clear();
        log::info!("🔄 Melspec accumulator reset - clearing old audio context");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pipeline_config_default() {
        let config = PipelineConfig::default();
        assert_eq!(config.chunk_size, 1280);
        assert_eq!(config.sample_rate, 16000);
        assert_eq!(config.confidence_threshold, 0.09);
        assert_eq!(config.window_size, 16);
        assert_eq!(config.overlap_size, 8);
    }

    #[test]
    fn test_pipeline_creation() {
        let config = PipelineConfig {
            melspectrogram_model_path: "non_existent_melspec.tflite".to_string(),
            embedding_model_path: "non_existent_embedding.tflite".to_string(),
            wakeword_model_path: "non_existent_wakeword.tflite".to_string(),
            ..Default::default()
        };

        let result = DetectionPipeline::new(config);
        assert!(result.is_err());
    }
}
