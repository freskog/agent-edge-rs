//! Audio feature extraction utilities for OpenWakeWord
//!
//! This module provides the AudioFeatures class that mimics the Python implementation,
//! including streaming audio processing and buffer management.

use crate::error::{OpenWakeWordError, Result};
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
    melspectrogram_buffer: Vec<Vec<f32>>, // [76, 32] buffer
    accumulated_samples: usize,
    raw_data_remainder: Vec<i16>,
    feature_buffer: Vec<Vec<f32>>, // Stores embeddings

    // Configuration
    feature_buffer_max_len: usize, // 120 frames (~10 seconds)
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

        // Use our XNNPACK-safe interpreter creation for melspec model
        let melspec_model =
            crate::xnnpack_fix::create_interpreter_with_xnnpack_safe(melspec_model_data, 1)
                .map_err(|e| {
                    OpenWakeWordError::ModelLoadError(format!(
                        "Failed to create melspec interpreter: {}",
                        e
                    ))
                })?;

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

        // Load embedding model
        let embedding_model_data = Box::leak(Box::new(
            TfliteModel::new(embedding_model_path).map_err(|e| {
                OpenWakeWordError::ModelLoadError(format!("Failed to load embedding model: {}", e))
            })?,
        ));

        // Use our XNNPACK-safe interpreter creation for embedding model
        let embedding_model =
            crate::xnnpack_fix::create_interpreter_with_xnnpack_safe(embedding_model_data, 1)
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
            melspectrogram_buffer: vec![vec![1.0; 32]; 76], // Initialize with ones like Python
            accumulated_samples: 0,
            raw_data_remainder: Vec::new(),
            feature_buffer: Vec::new(),
            feature_buffer_max_len: 120, // ~10 seconds
        };

        // Initialize feature buffer with dummy embeddings to avoid tensor allocation issues
        // We'll populate this properly when we start processing real audio
        instance.feature_buffer = vec![vec![0.0; 96]; 1]; // Start with one empty embedding

        Ok(instance)
    }

    /// Reset the internal buffers
    pub fn reset(&mut self) -> Result<()> {
        self.raw_data_buffer.clear();
        self.melspectrogram_buffer = vec![vec![1.0; 32]; 76];
        self.accumulated_samples = 0;
        self.raw_data_remainder.clear();

        // Reinitialize feature buffer with dummy embeddings
        self.feature_buffer = vec![vec![0.0; 96]; 1]; // Start with one empty embedding

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
            return self.feature_buffer[start_ndx as usize..end_ndx]
                .iter()
                .flat_map(|frame| frame.iter().copied())
                .collect();
        };

        // Extract frames from buffer
        let end_ndx = buffer_len; // Always go to end of buffer for negative indices
        let mut flattened = Vec::new();
        for i in actual_start_ndx..end_ndx {
            flattened.extend(&self.feature_buffer[i]);
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
                        self.melspectrogram_buffer.push(frame);
                        // Keep enough frames for multiple overlapping windows (e.g., 150 frames)
                        let max_frames = 150;
                        if self.melspectrogram_buffer.len() > max_frames {
                            self.melspectrogram_buffer.remove(0);
                        }
                    }

                    // Calculate embeddings from overlapping windows (match Python approach)
                    // Python extracts 76-frame windows with step size 8 from the melspectrogram buffer
                    if self.melspectrogram_buffer.len() >= 76 {
                        let step_size = 8;
                        let window_size = 76;

                        // Calculate how many windows we can extract
                        let max_start_idx = self.melspectrogram_buffer.len() - window_size;
                        let num_windows = (max_start_idx / step_size) + 1;

                        log::debug!(
                            "🔍 Extracting {} overlapping windows from {} frames (step_size={})",
                            num_windows,
                            self.melspectrogram_buffer.len(),
                            step_size
                        );

                        // Extract windows starting from the oldest (step_size intervals)
                        for window_idx in 0..num_windows {
                            let start_idx = window_idx * step_size;
                            let end_idx = start_idx + window_size;

                            if end_idx <= self.melspectrogram_buffer.len() {
                                log::debug!(
                                    "🔍 Creating embedding window {}: frames {}..{} (total frames: {})",
                                    window_idx, start_idx, end_idx, self.melspectrogram_buffer.len()
                                );

                                // Extract exactly 76 frames from melspectrogram buffer
                                let mut melspec_window = Vec::new();
                                for frame_idx in start_idx..end_idx {
                                    melspec_window.extend(&self.melspectrogram_buffer[frame_idx]);
                                }

                                // Compute embedding if we have exactly 76 frames (76 * 32 = 2432 elements)
                                if melspec_window.len() == 76 * 32 {
                                    log::debug!(
                                        "🔍 Computing embedding from 76 mel frames (window {})",
                                        window_idx
                                    );

                                    let embedding =
                                        self._get_embeddings_from_melspec(&melspec_window)?;
                                    log::debug!(
                                        "🔍 Got embedding with {} features",
                                        embedding.len()
                                    );

                                    // Update feature buffer (FIFO)
                                    self.feature_buffer.push(embedding);
                                    if self.feature_buffer.len() > self.feature_buffer_max_len {
                                        self.feature_buffer.remove(0);
                                    }
                                    log::debug!(
                                        "🔍 Feature buffer now has {} embeddings",
                                        self.feature_buffer.len()
                                    );
                                } else {
                                    log::debug!(
                                        "🔍 Window {} has {} elements, expected 2432",
                                        window_idx,
                                        melspec_window.len()
                                    );
                                }
                            }
                        }
                    } else {
                        log::debug!(
                            "🔍 Not enough frames for embedding: have {}, need {}",
                            self.melspectrogram_buffer.len(),
                            76
                        );
                    }
                }

                // Reset accumulated samples counter
                self.accumulated_samples = 0;
            }

            // Store remainder for next processing
            self.raw_data_remainder = remainder_data;
        } else {
            // Not enough samples yet, just accumulate
            self.accumulated_samples += combined_data.len();
            self.raw_data_remainder = combined_data;
        }

        Ok(processed_samples)
    }
}
