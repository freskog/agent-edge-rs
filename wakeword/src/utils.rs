//! Audio feature extraction utilities for OpenWakeWord
//!
//! This module provides the AudioFeatures class that mimics the Python implementation,
//! including streaming audio processing and buffer management.

use crate::error::{OpenWakeWordError, Result};
use rand::Rng;
use std::collections::VecDeque;
use tflitec::interpreter::Options;
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
    sample_rate: u32,
    melspectrogram_max_len: usize, // 10*97 frames
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

        let mut melspec_options = Options::default();
        melspec_options.thread_count = 1;

        let mut melspec_model = Interpreter::new(melspec_model_data, Some(melspec_options))
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

        let mut embedding_options = Options::default();
        embedding_options.thread_count = 1;

        let mut embedding_model = Interpreter::new(embedding_model_data, Some(embedding_options))
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
            sample_rate: sr,
            melspectrogram_max_len: 10 * 97, // 97 frames per second
            feature_buffer_max_len: 120,     // ~10 seconds
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

    /// Get features for model prediction
    ///
    /// # Arguments  
    /// * `n_feature_frames` - Number of feature frames to return (default: 16)
    /// * `start_ndx` - Starting index in buffer (-1 for most recent)
    ///
    /// # Returns
    /// * Flattened feature vector for model input
    pub fn get_features(&self, n_feature_frames: usize, start_ndx: i32) -> Vec<f32> {
        let buffer_len = self.feature_buffer.len();

        let actual_start_ndx = if start_ndx == -1 {
            if buffer_len >= n_feature_frames {
                buffer_len.saturating_sub(n_feature_frames)
            } else {
                0
            }
        } else {
            (start_ndx as usize).min(buffer_len)
        };

        let end_ndx = (actual_start_ndx + n_feature_frames).min(buffer_len);

        // Flatten the features from the buffer
        let mut flattened = Vec::new();
        for i in actual_start_ndx..end_ndx {
            flattened.extend(&self.feature_buffer[i]);
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

        // Apply transform: x/10 + 2 (matching Python melspec_transform)
        let transformed: Vec<f32> = output_data.iter().map(|&x| x / 10.0 + 2.0).collect();

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
        // Combine remainder with new data
        let mut combined_data = self.raw_data_remainder.clone();
        combined_data.extend(x.iter().copied());

        let chunk_size = 1280; // 80ms at 16kHz
        let mut processed_samples: usize = 0;

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
                let spec = self._get_melspectrogram(mel_input)?;

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

                        // Add new frame to buffer (FIFO)
                        self.melspectrogram_buffer.push(frame);
                        if self.melspectrogram_buffer.len() > 76 {
                            self.melspectrogram_buffer.remove(0);
                        }
                    }

                    // Check if we have enough frames for embedding (exactly 76 frames)
                    if self.melspectrogram_buffer.len() == 76 {
                        // Flatten exactly 76 frames for embedding model: 76 * 32 = 2432 elements
                        let flattened: Vec<f32> = self
                            .melspectrogram_buffer
                            .iter()
                            .flat_map(|frame| frame.iter().copied())
                            .collect();

                        // Get embedding (should output 96 features)
                        let embedding = self._get_embeddings_from_melspec(&flattened)?;

                        // Update feature buffer (FIFO)
                        self.feature_buffer.push(embedding);
                        if self.feature_buffer.len() > self.feature_buffer_max_len {
                            self.feature_buffer.remove(0);
                        }
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
