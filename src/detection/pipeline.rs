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
    pub confidence_threshold: f32, // Detection threshold: 0.3 (30% confidence)

    /// Windowing parameters  
    pub window_size: usize, // Embedding window size: 16 embeddings = ~1.28s context
    pub overlap_size: usize, // Overlap for sliding windows (currently unused)

    /// Debouncing parameters
    pub debounce_duration_ms: u64, // Minimum time between detections: 1000ms (1 second)

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
            confidence_threshold: 0.3, // Lower threshold for better real-world performance
            window_size: 16,
            overlap_size: 8,
            debounce_duration_ms: 1000, // 1 second debounce (OpenWakeWord uses 1.25s in tests)
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
///    - Stores the last 16 embedding vectors (96 features each)
///    - Provides 1.28 seconds of semantic context for wake word classification
///    - Implements proper sliding window behavior for real-time detection
///
/// ## Memory Management
///
/// Both windows have fixed maximum sizes to prevent unbounded memory growth:
/// - Melspec accumulator: ~16 × 160 = 2,560 f32 values (~10KB)
/// - Embedding window: 16 × 96 = 1,536 f32 values (~6KB)
/// - Total pipeline memory footprint: <100KB (excluding model weights)
pub struct DetectionPipeline {
    // Core ML models for the three-stage pipeline
    melspectrogram_model: MelSpectrogramModel, // Stage 1: Audio → Mel features
    embedding_model: EmbeddingModel,           // Stage 2: Mel features → Embeddings
    wakeword_model: WakewordModel,             // Stage 3: Embeddings → Detection
    config: PipelineConfig,                    // Pipeline configuration

    // Rolling window for embedding features (16 embeddings × 96 features each)
    // This represents ~1.28 seconds of audio context needed for wake word classification
    embedding_window: VecDeque<Vec<f32>>,

    // Accumulator for melspectrogram features (need 76 frames for embedding)
    // Collects melspec outputs until we have sufficient temporal context
    melspec_accumulator: VecDeque<Vec<f32>>,
    melspec_frames_needed: usize, // How many melspec outputs we need for one embedding input

    // Simple detection gating - much cleaner than complex silence gap detection
    // After a detection, ignore subsequent detections until confidence drops below threshold
    ignore_detections: bool,

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
            embedding_window,
            melspec_accumulator,
            melspec_frames_needed,
            ignore_detections,
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

        // ═══════════════════════════════════════════════════════════════════════════════════
        // STEP 1: MELSPECTROGRAM FEATURE EXTRACTION
        // ═══════════════════════════════════════════════════════════════════════════════════
        // Convert raw audio (1280 samples) into mel-scale frequency features (160 features)
        //
        // Input:  [1280] f32 samples (80ms at 16kHz)
        // Output: [1, 1, 5, 32] = 160 features (5 time frames × 32 mel bins)
        //
        // The melspectrogram represents audio in a perceptually meaningful way, similar to
        // how the human auditory system processes sound. Each of the 5 time frames represents
        // ~16ms of audio, and the 32 mel bins capture frequency content from low to high.
        let melspec_features = self.melspectrogram_model.predict(audio_chunk)?;
        log::debug!(
            "✓ Melspectrogram: {} samples → {} mel features (5 frames × 32 bins)",
            audio_chunk.len(),
            melspec_features.len()
        );

        // ═══════════════════════════════════════════════════════════════════════════════════
        // STEP 2: MELSPECTROGRAM ACCUMULATION
        // ═══════════════════════════════════════════════════════════════════════════════════
        // Accumulate melspectrogram outputs in a sliding window to build sufficient temporal
        // context for the embedding model. We need 76 consecutive mel frames, so we collect
        // ~16 melspec outputs (16 × 5 = 80 frames) and take the most recent 76.
        self.melspec_accumulator.push_back(melspec_features);

        // Check if accumulator has grown too large (2x needed size)
        if self.melspec_accumulator.len() > self.melspec_frames_needed * 2 {
            log::warn!("🔄 Melspec accumulator too large - resetting to prevent stall");
            self.reset_melspec_accumulator();
            return Ok(WakewordDetection {
                detected: false,
                confidence: 0.0,
                timestamp: std::time::Instant::now(),
            });
        }

        if self.melspec_accumulator.len() > self.melspec_frames_needed {
            self.melspec_accumulator.pop_front(); // Maintain sliding window
        }

        // ═══════════════════════════════════════════════════════════════════════════════════
        // STEP 3: CHECK MELSPECTROGRAM READINESS
        // ═══════════════════════════════════════════════════════════════════════════════════
        // Ensure we have enough melspectrogram outputs to proceed. During startup, we need
        // to accumulate sufficient context before meaningful embedding generation is possible.
        if self.melspec_accumulator.len() < self.melspec_frames_needed {
            log::debug!(
                "⏳ Accumulating melspec context: {}/{} outputs (need {}×5={} frames for embedding)",
                self.melspec_accumulator.len(),
                self.melspec_frames_needed,
                self.melspec_frames_needed,
                self.melspec_frames_needed * 5
            );
            return Ok(WakewordDetection {
                detected: false,
                confidence: 0.0,
                timestamp: std::time::Instant::now(),
            });
        }

        // ═══════════════════════════════════════════════════════════════════════════════════
        // STEP 4: PREPARE EMBEDDING INPUT
        // ═══════════════════════════════════════════════════════════════════════════════════
        // Reshape accumulated melspectrogram features for the embedding model.
        //
        // Process: Flatten all melspec outputs → Extract 76 most recent frames → Pad if needed
        // Target:  [1, 76, 32, 1] = 2432 features for embedding model input
        //
        // Why 76 frames? This provides ~1.5 seconds of audio context (76 × 20ms = 1.52s),
        // which is optimal for capturing phonetic patterns while remaining computationally efficient.
        let flattened_melspecs: Vec<f32> =
            self.melspec_accumulator.iter().flatten().cloned().collect();

        // Calculate frame extraction: each frame is 32 mel features
        let total_frames = flattened_melspecs.len() / 32;
        let start_frame = if total_frames >= 76 {
            total_frames - 76 // Take most recent 76 frames
        } else {
            0 // Use all available frames if less than 76
        };

        // Extract the target frame range
        let end_frame = (start_frame + 76).min(total_frames);
        let embedding_input: Vec<f32> =
            flattened_melspecs[start_frame * 32..end_frame * 32].to_vec();

        // Zero-pad to ensure consistent 2432-feature input (76 × 32)
        // During startup, this ensures the model receives properly sized input
        let mut padded_input = vec![0.0f32; 2432];
        let copy_len = embedding_input.len().min(2432);
        padded_input[2432 - copy_len..].copy_from_slice(&embedding_input[..copy_len]);

        log::debug!(
            "✓ Embedding prep: {} melspec outputs → {} total frames → {} target frames → {} features",
            self.melspec_accumulator.len(),
            total_frames,
            end_frame - start_frame,
            padded_input.len()
        );

        // ═══════════════════════════════════════════════════════════════════════════════════
        // STEP 5: EMBEDDING GENERATION
        // ═══════════════════════════════════════════════════════════════════════════════════
        // Generate semantic embeddings from melspectrogram features.
        //
        // Input:  [1, 76, 32, 1] = 2432 mel features (76 time frames × 32 mel bins)
        // Output: [1, 1, 1, 96] = 96 embedding features
        //
        // The embedding model transforms raw acoustic features into a dense semantic
        // representation that captures phonetic and linguistic patterns relevant for
        // wake word detection. This abstraction makes the system more robust to speaker
        // variations, accents, and background noise.
        let embedding_features = self.embedding_model.predict(&padded_input)?;
        log::debug!(
            "✓ Embedding: {} mel features → {} embedding features",
            padded_input.len(),
            embedding_features.len()
        );

        // ═══════════════════════════════════════════════════════════════════════════════════
        // STEP 6: EMBEDDING WINDOW MANAGEMENT
        // ═══════════════════════════════════════════════════════════════════════════════════
        // Maintain a sliding window of embeddings for wake word detection. We need 16
        // consecutive embeddings (representing ~1.28 seconds of audio) to provide sufficient
        // temporal context for accurate wake word classification.
        self.embedding_window.push_back(embedding_features);

        // Check if window has grown too large (2x needed size)
        if self.embedding_window.len() > self.config.window_size * 2 {
            log::warn!("🔄 Embedding window too large - resetting to prevent stall");
            self.embedding_window.clear();
            return Ok(WakewordDetection {
                detected: false,
                confidence: 0.0,
                timestamp: std::time::Instant::now(),
            });
        }

        if self.embedding_window.len() > self.config.window_size {
            self.embedding_window.pop_front(); // Maintain fixed window size
        }

        // ═══════════════════════════════════════════════════════════════════════════════════
        // STEP 7: CHECK EMBEDDING READINESS
        // ═══════════════════════════════════════════════════════════════════════════════════
        // Always try to detect with whatever embeddings we have (minimum 1)
        // Zero-padding will handle cases where we have fewer than 16 embeddings
        if self.embedding_window.is_empty() {
            log::debug!("⏳ No embeddings yet - waiting for first embedding");
            return Ok(WakewordDetection {
                detected: false,
                confidence: 0.0,
                timestamp: std::time::Instant::now(),
            });
        }

        // ═══════════════════════════════════════════════════════════════════════════════════
        // STEP 8: PREPARE WAKE WORD INPUT
        // ═══════════════════════════════════════════════════════════════════════════════════
        // Flatten the embedding window for wake word model input, padding if necessary.
        //
        // Target: 16 embeddings × 96 features = 1536 total features
        // Always zero-pad to full size - this allows detection even with just 1 embedding
        let mut flattened_embeddings: Vec<f32> =
            self.embedding_window.iter().flatten().cloned().collect();

        // Ensure we have exactly 1536 features for the model (16 × 96)
        let target_size = self.config.window_size * 96; // 16 × 96 = 1536
        if flattened_embeddings.len() < target_size {
            // Pad with zeros at the beginning (older time steps)
            let padding_needed = target_size - flattened_embeddings.len();
            let mut padded = vec![0.0f32; padding_needed];
            padded.extend(flattened_embeddings);
            flattened_embeddings = padded;
        }

        log::debug!(
            "✓ Wake word prep: {} embeddings × 96 features = {} total features (zero-padded to {})",
            self.embedding_window.len(),
            self.embedding_window.len() * 96,
            flattened_embeddings.len()
        );

        // ═══════════════════════════════════════════════════════════════════════════════════
        // STEP 9: WAKE WORD DETECTION
        // ═══════════════════════════════════════════════════════════════════════════════════
        // Run the final classification to determine if the target wake word is present.
        //
        // Input:  [1, 16, 96] = 1536 embedding features (16 time steps × 96 features)
        // Output: [1, 1] = single confidence score (0.0 - 1.0)
        //
        // The wake word model is specifically trained on "Hey Mycroft" variations and
        // produces a confidence score indicating the likelihood that the target phrase
        // is present in the current audio window.
        let confidence = self.wakeword_model.predict(&flattened_embeddings)?;
        let above_threshold = confidence >= self.config.confidence_threshold;

        // Always log confidence for debugging
        log::debug!(
            "🎯 Wake word confidence: {:.4} (threshold: {:.2})",
            confidence,
            self.config.confidence_threshold
        );

        // ═══════════════════════════════════════════════════════════════════════════════════
        // STEP 10: SIMPLE DETECTION GATING
        // ═══════════════════════════════════════════════════════════════════════════════════
        // Use simple flag-based gating: after detection, ignore until confidence drops below threshold
        let mut detected = false;
        let now = std::time::Instant::now();

        if above_threshold && !self.ignore_detections {
            // Above threshold and not ignoring - allow detection
            detected = true;
            self.ignore_detections = true; // Set ignore flag to prevent immediate re-detection
            log::info!("🎉 WAKEWORD DETECTED! Confidence: {:.3}", confidence);

            // Trigger LED feedback for wake word detection
            if let Some(ref led_ring) = self.led_ring {
                if let Err(e) = led_ring.set_color(
                    self.config.led_detected_color.0,
                    self.config.led_detected_color.1,
                    self.config.led_detected_color.2,
                ) {
                    log::warn!("Failed to set LED detection color: {}", e);
                }
            }
        } else if above_threshold && self.ignore_detections {
            // Above threshold but ignoring detections - suppress
            log::debug!(
                "🔇 Detection suppressed: confidence {:.3} (ignoring detections)",
                confidence
            );
        } else if !above_threshold && self.ignore_detections {
            // Below threshold and currently ignoring - reset ignore flag
            self.ignore_detections = false;
            log::debug!(
                "🔄 Confidence dropped to {:.3} - ready for next detection",
                confidence
            );
        } else {
            // Below threshold and not ignoring - normal state
            log::debug!(
                "📊 Detection confidence: {:.4} (below threshold)",
                confidence
            );
        }

        Ok(WakewordDetection {
            detected,
            confidence,
            timestamp: now,
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
        log::debug!("💡 LED reset to listening mode");
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
        assert_eq!(config.confidence_threshold, 0.3);
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
