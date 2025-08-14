//! Audio feature extraction utilities for OpenWakeWord
//!
//! This module provides the AudioFeatures class that mimics the Python implementation,
//! including streaming audio processing and buffer management.

use crate::wakeword_error::{OpenWakeWordError, Result};
use std::collections::VecDeque;
use tflitec::tensor::Shape;
use tflitec::{interpreter::Interpreter, model::Model as TfliteModel};

/// AudioFeatures class for creating audio features from audio data
///
/// This matches the Python AudioFeatures class interface:
/// - Call `audio_features(audio_data)` to process audio and get number of prepared samples
/// - Call `audio_features.get_features(n_frames, start_ndx)` to extract features
pub struct AudioFeatures {
    // TensorFlow Lite models
    melspec_model: Interpreter<'static>,
    embedding_model: Interpreter<'static>,

    // Streaming buffers (matching Python implementation)
    raw_data_buffer: VecDeque<i16>,
    melspectrogram_buffer: VecDeque<Vec<f32>>, // [76, 32] buffer
    accumulated_samples: usize,
    raw_data_remainder: Vec<i16>,
    feature_buffer: VecDeque<Vec<f32>>, // Stores embeddings

    // Configuration
    feature_buffer_max_len: usize, // 120 frames (~10 seconds)

    // Track processed chunks to avoid recomputation (like Python)
    processed_chunks: usize, // Number of chunks we've already computed embeddings for
}

impl AudioFeatures {
    /// Create a new AudioFeatures instance
    ///
    /// # Arguments
    /// * `melspec_model_path` - Path to melspectrogram model
    /// * `embedding_model_path` - Path to embedding model  
    /// * `sr` - Sample rate (default: 16000)
    pub fn new(melspec_model_path: &str, embedding_model_path: &str, sr: u32) -> Result<Self> {
        // Load melspectrogram model
        let melspec_model_data = Box::leak(Box::new(
            TfliteModel::new(melspec_model_path).map_err(|e| {
                OpenWakeWordError::ModelLoadError(format!("Failed to load melspec model: {}", e))
            })?,
        ));

        // Match Python's default single-threaded configuration for stability
        let mut options = tflitec::interpreter::Options::default();
        options.thread_count = 1; // Match Python's default (ncpu=1)
        options.is_xnnpack_enabled = true; // Keep XNNPACK for performance

        let melspec_model =
            tflitec::interpreter::Interpreter::new(melspec_model_data, Some(options)).map_err(
                |e| {
                    OpenWakeWordError::ModelLoadError(format!(
                        "Failed to create melspec interpreter: {}",
                        e
                    ))
                },
            )?;

        // Resize melspec input tensor to a reasonable shape before allocating tensors
        // This avoids the integer overflow issue with the default model shape
        let melspec_input_shape = Shape::new(vec![1, 1280]); // 80ms at 16kHz
        melspec_model
            .resize_input(0, melspec_input_shape)
            .map_err(|e| {
                OpenWakeWordError::ModelLoadError(format!("Failed to resize melspec input: {}", e))
            })?;

        melspec_model.allocate_tensors().map_err(|e| {
            OpenWakeWordError::ModelLoadError(format!("Failed to allocate melspec tensors: {}", e))
        })?;

        // Load embedding model with multi-core optimization
        let embedding_model_data = Box::leak(Box::new(
            TfliteModel::new(embedding_model_path).map_err(|e| {
                OpenWakeWordError::ModelLoadError(format!("Failed to load embedding model: {}", e))
            })?,
        ));

        // Match Python's single-threaded configuration for both models
        let mut embedding_options = tflitec::interpreter::Options::default();
        embedding_options.thread_count = 1; // Match Python's default (ncpu=1)
        embedding_options.is_xnnpack_enabled = true; // Keep XNNPACK for performance

        let embedding_model =
            tflitec::interpreter::Interpreter::new(embedding_model_data, Some(embedding_options))
                .map_err(|e| {
                OpenWakeWordError::ModelLoadError(format!(
                    "Failed to create embedding interpreter: {}",
                    e
                ))
            })?;

        // Resize embedding input tensor to the correct shape: [1, 76, 32, 1]
        // This matches Python: self.embedding_model.resize_tensor_input(0, [1, 76, 32, 1], strict=True)
        let embedding_input_shape = Shape::new(vec![1, 76, 32, 1]);
        embedding_model
            .resize_input(0, embedding_input_shape)
            .map_err(|e| {
                OpenWakeWordError::ModelLoadError(format!(
                    "Failed to resize embedding input: {}",
                    e
                ))
            })?;

        embedding_model.allocate_tensors().map_err(|e| {
            OpenWakeWordError::ModelLoadError(format!(
                "Failed to allocate embedding tensors: {}",
                e
            ))
        })?;

        // Initialize buffers (matching Python implementation)
        let mut instance = AudioFeatures {
            melspec_model,
            embedding_model,
            raw_data_buffer: VecDeque::with_capacity(sr as usize * 10), // 10 seconds
            melspectrogram_buffer: {
                let mut buf = VecDeque::with_capacity(970);
                // Initialize with ones like Python - preserve exact original logic
                for _ in 0..76 {
                    buf.push_back(vec![1.0; 32]);
                }
                buf
            },
            accumulated_samples: 0,
            raw_data_remainder: Vec::new(),
            feature_buffer: VecDeque::new(),
            feature_buffer_max_len: 120, // ~10 seconds
            processed_chunks: 0,
        };

        // Initialize feature buffer with dummy embeddings to avoid tensor allocation issues
        // We'll populate this properly when we start processing real audio
        instance.feature_buffer.push_back(vec![0.0; 96]); // Start with one empty embedding

        Ok(instance)
    }

    /// Reset the internal buffers
    pub fn reset(&mut self) -> Result<()> {
        self.raw_data_buffer.clear();
        self.melspectrogram_buffer.clear();
        for _ in 0..76 {
            self.melspectrogram_buffer.push_back(vec![1.0; 32]);
        }
        self.accumulated_samples = 0;
        self.raw_data_remainder.clear();
        self.processed_chunks = 0; // Reset processed chunks counter

        // Reinitialize feature buffer with dummy embeddings
        self.feature_buffer.clear();
        self.feature_buffer.push_back(vec![0.0; 96]); // Start with one empty embedding

        Ok(())
    }

    /// Process audio data (main callable interface like Python)
    ///
    /// # Arguments
    /// * `x` - Input audio data (16-bit PCM)
    ///
    /// # Returns
    /// * Number of prepared samples
    pub fn __call__(&mut self, x: &[i16]) -> Result<usize> {
        self._streaming_features(x)
    }

    /// Get features for model prediction (matches Python get_features)
    ///
    /// # Arguments  
    /// * `n_feature_frames` - Number of feature frames to return (default: 16)
    /// * `start_ndx` - Starting index in buffer (-1 for most recent)
    ///
    /// # Returns
    /// * Flattened feature vector for model input
    pub fn get_features(&self, n_feature_frames: usize, start_ndx: i32) -> Vec<f32> {
        let buffer_len = self.feature_buffer.len();

        // If buffer is empty, return empty vector
        if buffer_len == 0 {
            return Vec::new();
        }

        // Match Python's approach: self.feature_buffer[int(-1*n_feature_frames):, :]
        let actual_start_ndx = if start_ndx < 0 {
            // Python: self.feature_buffer[int(-1*n_feature_frames):, :]
            // This takes the last n_feature_frames, even if buffer is shorter
            if buffer_len >= n_feature_frames {
                buffer_len - n_feature_frames
            } else {
                0 // Take all available frames if buffer is shorter
            }
        } else {
            let end_ndx = (start_ndx as usize + n_feature_frames).min(buffer_len);
            let buffer_vec: Vec<_> = self.feature_buffer.iter().collect();
            return buffer_vec[start_ndx as usize..end_ndx]
                .iter()
                .flat_map(|frame| frame.iter().copied())
                .collect();
        };

        // Extract frames from buffer
        let end_ndx = buffer_len; // Always go to end of buffer for negative indices
        let mut flattened = Vec::new();
        let buffer_vec: Vec<_> = self.feature_buffer.iter().collect();
        for i in actual_start_ndx..end_ndx {
            flattened.extend(buffer_vec[i]);
        }

        // If we have more features than requested, take only the last n_feature_frames worth
        if flattened.len() > n_feature_frames * 96 {
            let start_feature_idx = flattened.len() - (n_feature_frames * 96);
            flattened = flattened[start_feature_idx..].to_vec();
        }

        flattened
    }

    /// Compute melspectrogram from audio data
    fn _get_melspectrogram(&mut self, x: &[i16]) -> Result<Vec<f32>> {
        // Convert to float and reshape
        let audio_f32: Vec<f32> = x.iter().map(|&sample| sample as f32).collect();

        // Resize input tensor to match audio length (like Python implementation)
        let input_shape = Shape::new(vec![1, audio_f32.len()]);
        self.melspec_model
            .resize_input(0, input_shape)
            .map_err(|e| {
                OpenWakeWordError::ProcessingError(format!("Failed to resize melspec input: {}", e))
            })?;

        // Reallocate tensors after resize
        self.melspec_model.allocate_tensors().map_err(|e| {
            OpenWakeWordError::ProcessingError(format!(
                "Failed to reallocate melspec tensors: {}",
                e
            ))
        })?;

        // Set input tensor
        self.melspec_model.copy(&audio_f32, 0).map_err(|e| {
            OpenWakeWordError::ProcessingError(format!("Failed to set melspec input: {}", e))
        })?;

        // Run inference
        self.melspec_model.invoke().map_err(|e| {
            OpenWakeWordError::ProcessingError(format!("Melspec inference failed: {}", e))
        })?;

        // Get output
        let output_tensor = self.melspec_model.output(0).map_err(|e| {
            OpenWakeWordError::ProcessingError(format!("Failed to get melspec output: {}", e))
        })?;

        let output_data = output_tensor.data::<f32>().to_vec();

        // Debug: print raw melspectrogram output
        log::debug!(
            "🔍 Rust Stage 1 - Melspec input: {} samples, output raw: {} elements",
            audio_f32.len(),
            output_data.len()
        );
        if !output_data.is_empty() {
            let mean = output_data.iter().sum::<f32>() / output_data.len() as f32;
            let min = output_data.iter().fold(f32::INFINITY, |a, &b| a.min(b));
            let max = output_data.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b));
            log::debug!(
                "🔍 Rust Stage 1 - Melspec raw stats: mean={:.6}, min={:.6}, max={:.6}",
                mean,
                min,
                max
            );
        }

        // Apply transform: x/10 + 2 (matching Python melspec_transform)
        let transformed: Vec<f32> = output_data.iter().map(|&x| x / 10.0 + 2.0).collect();

        // Debug: print transformed melspectrogram output
        if !transformed.is_empty() {
            let mean = transformed.iter().sum::<f32>() / transformed.len() as f32;
            let min = transformed.iter().fold(f32::INFINITY, |a, &b| a.min(b));
            let max = transformed.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b));
            log::debug!(
                "🔍 Rust Stage 1 - Melspec after transform: mean={:.6}, min={:.6}, max={:.6}",
                mean,
                min,
                max
            );
            log::debug!(
                "🔍 Rust Stage 1 - First 5 values: {:?}",
                &transformed[..transformed.len().min(5)]
            );
        }

        Ok(transformed)
    }

    /// Compute embeddings from melspectrogram
    /// Expects melspec to be 76*32=2432 elements that will be reshaped to [1, 76, 32, 1]
    fn _get_embeddings_from_melspec(&mut self, melspec: &[f32]) -> Result<Vec<f32>> {
        // Verify input size matches expected: 76 frames × 32 features = 2432 elements
        if melspec.len() != 76 * 32 {
            return Err(OpenWakeWordError::ProcessingError(format!(
                "Embedding model expects 76*32=2432 mel features, got {}",
                melspec.len()
            )));
        }

        // Debug: print embedding input
        log::debug!(
            "🔍 Rust Stage 2 - Embedding input: {} elements (76×32)",
            melspec.len()
        );
        if !melspec.is_empty() {
            let mean = melspec.iter().sum::<f32>() / melspec.len() as f32;
            let min = melspec.iter().fold(f32::INFINITY, |a, &b| a.min(b));
            let max = melspec.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b));
            log::debug!(
                "🔍 Rust Stage 2 - Embedding input stats: mean={:.6}, min={:.6}, max={:.6}",
                mean,
                min,
                max
            );
        }

        // The tensor is already resized to [1, 76, 32, 1] during initialization
        // We just need to copy the flattened data which TensorFlow Lite will interpret correctly
        self.embedding_model.copy(melspec, 0).map_err(|e| {
            OpenWakeWordError::ProcessingError(format!("Failed to set embedding input: {}", e))
        })?;

        // Run inference
        self.embedding_model.invoke().map_err(|e| {
            OpenWakeWordError::ProcessingError(format!("Embedding inference failed: {}", e))
        })?;

        // Get output
        let output_tensor = self.embedding_model.output(0).map_err(|e| {
            OpenWakeWordError::ProcessingError(format!("Failed to get embedding output: {}", e))
        })?;

        let output_data = output_tensor.data::<f32>().to_vec();

        // Debug: print embedding output
        log::debug!(
            "🔍 Rust Stage 2 - Embedding output: {} elements",
            output_data.len()
        );
        if !output_data.is_empty() {
            let mean = output_data.iter().sum::<f32>() / output_data.len() as f32;
            let min = output_data.iter().fold(f32::INFINITY, |a, &b| a.min(b));
            let max = output_data.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b));
            log::debug!(
                "🔍 Rust Stage 2 - Embedding output stats: mean={:.6}, min={:.6}, max={:.6}",
                mean,
                min,
                max
            );
            log::debug!(
                "🔍 Rust Stage 2 - First 5 values: {:?}",
                &output_data[..output_data.len().min(5)]
            );
        }

        Ok(output_data)
    }

    /// Compute embeddings from raw audio (matches Python _get_embeddings)
    fn _get_embeddings(&mut self, x: &[i16]) -> Result<Vec<Vec<f32>>> {
        let window_size = 76;
        let step_size = 8;

        // Get melspectrogram
        let spec = self._get_melspectrogram(x)?;

        // Create windows with step size 8
        let mut windows = Vec::new();
        for i in (0..spec.len()).step_by(step_size) {
            if i + window_size * 32 <= spec.len() {
                let window = &spec[i..i + window_size * 32];
                windows.push(window.to_vec());
            }
        }

        if windows.is_empty() {
            return Ok(vec![vec![0.0; 96]]); // Return default if no windows
        }

        // Process each window to get embeddings
        let mut all_embeddings = Vec::new();
        for window in windows {
            let embedding = self._get_embeddings_from_melspec(&window)?;
            all_embeddings.push(embedding);
        }

        Ok(all_embeddings)
    }

    /// Buffer raw audio data for streaming
    fn _buffer_raw_data(&mut self, x: &[i16]) {
        self.raw_data_buffer.extend(x.iter().copied());

        // Trim buffer to prevent memory leak - keep only last 10 seconds of audio
        // Each sample is 1/16000 second, so 10 seconds = 160,000 samples
        const MAX_RAW_BUFFER_SAMPLES: usize = 160_000; // 10 seconds at 16kHz

        if self.raw_data_buffer.len() > MAX_RAW_BUFFER_SAMPLES {
            let excess = self.raw_data_buffer.len() - MAX_RAW_BUFFER_SAMPLES;
            for _ in 0..excess {
                self.raw_data_buffer.pop_front();
            }
            log::debug!(
                "🔧 Trimmed raw_data_buffer: removed {} samples, now {} samples ({:.1}s)",
                excess,
                self.raw_data_buffer.len(),
                self.raw_data_buffer.len() as f64 / 16000.0
            );
        }

        // Note: accumulated_samples is managed separately in _streaming_features
    }

    /// Process streaming audio features (matches Python _streaming_features)  
    fn _streaming_features(&mut self, x: &[i16]) -> Result<usize> {
        log::debug!(
            "🔍 AudioFeatures::_streaming_features - input: {} samples",
            x.len()
        );
        log::debug!(
            "🔍 Current state: accumulated_samples={}, remainder={}, buffer_len={}",
            self.accumulated_samples,
            self.raw_data_remainder.len(),
            self.raw_data_buffer.len()
        );
        log::debug!(
            "🔍 Feature buffer has {} embeddings",
            self.feature_buffer.len()
        );

        // Combine remainder with new data
        let mut combined_data = self.raw_data_remainder.clone();
        combined_data.extend(x.iter().copied());

        let chunk_size = 1280; // 80ms at 16kHz
        let mut processed_samples: usize = 0;

        log::debug!(
            "🔍 Combined data: {} samples, need {} for processing",
            combined_data.len(),
            chunk_size
        );

        // Only process if we have enough samples for mel computation
        if self.accumulated_samples + combined_data.len() >= chunk_size {
            let remainder = (self.accumulated_samples + combined_data.len()) % chunk_size;

            let (process_data, remainder_data) = if remainder != 0 {
                let split_point = combined_data.len() - remainder;
                (
                    combined_data[..split_point].to_vec(),
                    combined_data[split_point..].to_vec(),
                )
            } else {
                (combined_data.clone(), Vec::new())
            };

            // Add processed data to buffer and update accumulated samples
            self._buffer_raw_data(&process_data);
            self.accumulated_samples += process_data.len();
            processed_samples = process_data.len();

            // Process in chunks and update melspectrogram buffer
            if self.accumulated_samples >= chunk_size && self.accumulated_samples % chunk_size == 0
            {
                log::debug!(
                    "🔍 Processing mel chunk: accumulated_samples={}",
                    self.accumulated_samples
                );

                // Get melspectrogram for the accumulated samples
                let buffer_data: Vec<i16> = self.raw_data_buffer.iter().copied().collect();
                let n_samples = self.accumulated_samples;

                // Extract the most recent n_samples + some padding for melspec computation
                let start_idx = if buffer_data.len() > n_samples + 480 {
                    // 480 = 160*3 (3 frames of padding)
                    buffer_data.len() - n_samples - 480
                } else {
                    0
                };

                let mel_input = &buffer_data[start_idx..];
                log::debug!("🔍 Getting melspectrogram from {} samples", mel_input.len());
                let spec = self._get_melspectrogram(mel_input)?;
                log::debug!("🔍 Got melspectrogram with {} elements", spec.len());

                // The melspectrogram output should be reshaped to frames
                // Assuming each 1280 samples produces 5 mel frames of 32 features each
                let frames_per_chunk = 5;
                let features_per_frame = 32;

                if spec.len() >= frames_per_chunk * features_per_frame {
                    // Add new frames to melspectrogram buffer
                    for i in 0..frames_per_chunk {
                        let start_idx = i * features_per_frame;
                        let end_idx = (i + 1) * features_per_frame;
                        let frame = spec[start_idx..end_idx].to_vec();

                        // Add new frame to buffer (keep more frames for sliding windows)
                        self.melspectrogram_buffer.push_back(frame);

                        // Calculate buffer size to match Python implementation exactly
                        // Python: melspectrogram_max_len = 10*97 = 970 frames (10 seconds)
                        // 97 frames = 1 second of 16kHz audio processed as melspectrograms
                        let max_frames = 970; // Match Python's buffer size exactly

                        if self.melspectrogram_buffer.len() > max_frames {
                            let dropped_frame_ms = (160.0 / 16000.0 * 1000.0) as u32; // Each frame = 160 samples = 10ms at 16kHz
                            log::debug!(
                                "⚠️ MELSPEC BUFFER OVERFLOW: Dropping oldest frame ({}ms of data) - buffer was {} frames", 
                                dropped_frame_ms, self.melspectrogram_buffer.len()
                            );
                            self.melspectrogram_buffer.pop_front();
                            log::debug!(
                                "🔧 Melspec buffer trimmed to {} frames (max: {})",
                                self.melspectrogram_buffer.len(),
                                max_frames
                            );
                        }
                    }

                    // Calculate embeddings from NEW chunks only (match Python approach)
                    // Python extracts embeddings only for newly processed chunks
                    if self.melspectrogram_buffer.len() >= 76 {
                        let chunks_to_process = self.accumulated_samples / chunk_size;
                        let new_chunks = chunks_to_process - self.processed_chunks;

                        log::debug!(
                            "🔍 Processing {} new chunks (total processed: {} -> {})",
                            new_chunks,
                            self.processed_chunks,
                            chunks_to_process
                        );

                        // Only compute embeddings for NEW chunks (like Python)
                        for i in (0..new_chunks).rev() {
                            // Calculate the index offset (matches Python: ndx = -8*i)
                            let offset = 8 * i;
                            let end_idx = self.melspectrogram_buffer.len() - offset;
                            let start_idx = if end_idx >= 76 { end_idx - 76 } else { 0 };

                            if end_idx > start_idx && (end_idx - start_idx) == 76 {
                                log::debug!(
                                    "🔍 Computing embedding for chunk {} (frames {}..{})",
                                    i,
                                    start_idx,
                                    end_idx
                                );

                                // Extract exactly 76 frames from melspectrogram buffer
                                let mut melspec_window = Vec::new();
                                let buffer_vec: Vec<_> =
                                    self.melspectrogram_buffer.iter().collect();
                                for frame_idx in start_idx..end_idx {
                                    melspec_window.extend(buffer_vec[frame_idx]);
                                }

                                // Compute embedding if we have exactly 76 frames (76 * 32 = 2432 elements)
                                if melspec_window.len() == 76 * 32 {
                                    let embedding =
                                        self._get_embeddings_from_melspec(&melspec_window)?;
                                    log::debug!(
                                        "🔍 Got embedding with {} features for chunk {}",
                                        embedding.len(),
                                        i
                                    );

                                    // Update feature buffer (FIFO)
                                    self.feature_buffer.push_back(embedding);
                                    if self.feature_buffer.len() > self.feature_buffer_max_len {
                                        let dropped_embedding_ms =
                                            (8.0 * 160.0 / 16000.0 * 1000.0) as u32; // Each embedding represents ~80ms of audio
                                        log::debug!(
                                            "⚠️ FEATURE BUFFER OVERFLOW: Dropping oldest embedding (~{}ms of detection data) - buffer was {} embeddings",
                                            dropped_embedding_ms, self.feature_buffer.len()
                                        );
                                        self.feature_buffer.pop_front();
                                        log::debug!(
                                            "🔧 Feature buffer trimmed to {} embeddings (max: {})",
                                            self.feature_buffer.len(),
                                            self.feature_buffer_max_len
                                        );
                                    }
                                }
                            }
                        }

                        // Update processed chunks counter (like Python resets accumulated_samples)
                        self.processed_chunks = chunks_to_process;
                        log::debug!("🔍 Updated processed_chunks to {}", self.processed_chunks);
                    }

                    // Reset accumulated samples counter (like Python does)
                    processed_samples = self.accumulated_samples;
                    self.accumulated_samples = 0;
                    self.processed_chunks = 0; // Reset when we finish processing this batch
                }

                // Store remainder for next processing
                self.raw_data_remainder = remainder_data;
            }
        } else {
            // Not enough samples yet, just accumulate
            self.accumulated_samples += combined_data.len();
            self.raw_data_remainder = combined_data;
        }

        Ok(processed_samples)
    }
}
