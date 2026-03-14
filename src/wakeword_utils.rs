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
        };

        // Initialize feature buffer with embeddings from random noise (matches Python:
        // self.feature_buffer = self._get_embeddings(np.random.randint(-1000, 1000, 16000*4)))
        let warmup_noise = Self::generate_warmup_noise(16000 * 4);
        let warmup_embeddings = instance._get_embeddings(&warmup_noise)?;
        for emb in warmup_embeddings {
            instance.feature_buffer.push_back(emb);
        }

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

        // Reinitialize feature buffer with embeddings from random noise (matches Python)
        self.feature_buffer.clear();
        let warmup_noise = Self::generate_warmup_noise(16000 * 4);
        let warmup_embeddings = self._get_embeddings(&warmup_noise)?;
        for emb in warmup_embeddings {
            self.feature_buffer.push_back(emb);
        }

        Ok(())
    }

    /// Generate deterministic pseudo-random noise for model warmup.
    /// Matches Python: np.random.randint(-1000, 1000, n_samples).astype(np.int16)
    fn generate_warmup_noise(n_samples: usize) -> Vec<i16> {
        let mut state: u64 = 42;
        let mut samples = Vec::with_capacity(n_samples);
        for _ in 0..n_samples {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let value = ((state >> 33) as i32 % 2001 - 1000) as i16;
            samples.push(value);
        }
        samples
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
    /// Supports negative indexing like Python/numpy:
    /// - `start_ndx == -1`: returns the last `n_feature_frames` frames (default behavior)
    /// - `start_ndx < -1`: interprets as negative index from the end of the buffer
    ///   e.g., start_ndx=-17 with n_feature_frames=16 returns buffer[-17:-1]
    /// - `start_ndx >= 0`: interprets as positive index from the start
    pub fn get_features(&self, n_feature_frames: usize, start_ndx: i32) -> Vec<f32> {
        let buffer_len = self.feature_buffer.len();
        if buffer_len == 0 {
            return Vec::new();
        }

        let buffer_vec: Vec<_> = self.feature_buffer.iter().collect();

        if start_ndx == -1 {
            // Default: return the last n_feature_frames
            // Python: self.feature_buffer[int(-1*n_feature_frames):, :]
            let start = buffer_len.saturating_sub(n_feature_frames);
            return buffer_vec[start..buffer_len]
                .iter()
                .flat_map(|frame| frame.iter().copied())
                .collect();
        }

        if start_ndx < -1 {
            // Negative indexing (matches Python numpy negative slicing)
            // Python: end_ndx = start_ndx + n if (start_ndx + n) != 0 else len(buffer)
            let abs_start = (-start_ndx) as usize;
            let start = buffer_len.saturating_sub(abs_start);
            let end_offset = start_ndx + n_feature_frames as i32;
            let end = if end_offset == 0 {
                buffer_len
            } else if end_offset < 0 {
                buffer_len.saturating_sub((-end_offset) as usize)
            } else {
                (start + n_feature_frames).min(buffer_len)
            };

            if start >= end {
                return Vec::new();
            }
            return buffer_vec[start..end]
                .iter()
                .flat_map(|frame| frame.iter().copied())
                .collect();
        }

        // Positive indexing
        let start = (start_ndx as usize).min(buffer_len);
        let end = (start + n_feature_frames).min(buffer_len);
        buffer_vec[start..end]
            .iter()
            .flat_map(|frame| frame.iter().copied())
            .collect()
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
        let window_size_frames = 76;
        let step_size_frames = 8;
        let features_per_frame = 32;

        let spec = self._get_melspectrogram(x)?;
        let n_frames = spec.len() / features_per_frame;

        // Python: for i in range(0, spec.shape[0], 8): window = spec[i:i+76]
        let mut windows = Vec::new();
        let mut frame_idx = 0;
        while frame_idx + window_size_frames <= n_frames {
            let start = frame_idx * features_per_frame;
            let end = (frame_idx + window_size_frames) * features_per_frame;
            windows.push(spec[start..end].to_vec());
            frame_idx += step_size_frames;
        }

        if windows.is_empty() {
            return Ok(vec![vec![0.0; 96]]);
        }

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
            "🔍 _streaming_features: input={} samples, accumulated={}, remainder={}, buffer={}",
            x.len(),
            self.accumulated_samples,
            self.raw_data_remainder.len(),
            self.raw_data_buffer.len()
        );

        let mut processed_samples: usize = 0;

        // Prepend any remainder from previous call (matches Python)
        let x = if !self.raw_data_remainder.is_empty() {
            let mut combined = std::mem::take(&mut self.raw_data_remainder);
            combined.extend_from_slice(x);
            combined
        } else {
            x.to_vec()
        };

        let chunk_size = 1280;

        if self.accumulated_samples + x.len() >= chunk_size {
            let remainder = (self.accumulated_samples + x.len()) % chunk_size;

            if remainder != 0 {
                let split = x.len() - remainder;
                self._buffer_raw_data(&x[..split]);
                self.accumulated_samples += split;
                self.raw_data_remainder = x[split..].to_vec();
            } else {
                self._buffer_raw_data(&x);
                self.accumulated_samples += x.len();
                self.raw_data_remainder.clear();
            }
        } else {
            // Not enough for a full chunk yet; buffer and accumulate (matches Python)
            self.accumulated_samples += x.len();
            self._buffer_raw_data(&x);
        }

        // Process mel + embeddings when we have complete chunks
        if self.accumulated_samples >= chunk_size && self.accumulated_samples % chunk_size == 0 {
            // Compute streaming melspectrogram (matches Python _streaming_melspectrogram)
            let buffer_data: Vec<i16> = self.raw_data_buffer.iter().copied().collect();
            let n_samples = self.accumulated_samples;

            // Python: list(self.raw_data_buffer)[-n_samples-160*3:]
            let padding = 160 * 3; // 480 samples
            let start_idx = if buffer_data.len() > n_samples + padding {
                buffer_data.len() - n_samples - padding
            } else {
                0
            };

            let mel_input = &buffer_data[start_idx..];
            let spec = self._get_melspectrogram(mel_input)?;

            // Add ALL mel frames to buffer (matches Python np.vstack)
            let features_per_frame = 32;
            let total_frames = spec.len() / features_per_frame;

            for i in 0..total_frames {
                let start = i * features_per_frame;
                let end = start + features_per_frame;
                self.melspectrogram_buffer
                    .push_back(spec[start..end].to_vec());
            }

            // Trim to max length (Python: melspectrogram_max_len = 10*97 = 970)
            const MAX_FRAMES: usize = 970;
            while self.melspectrogram_buffer.len() > MAX_FRAMES {
                self.melspectrogram_buffer.pop_front();
            }

            log::debug!(
                "🔍 Added {} mel frames, buffer now {} frames",
                total_frames,
                self.melspectrogram_buffer.len()
            );

            // Compute embeddings for each new chunk
            // Python: for i in np.arange(accumulated_samples//1280-1, -1, -1)
            if self.melspectrogram_buffer.len() >= 76 {
                let new_chunks = self.accumulated_samples / chunk_size;

                for i in (0..new_chunks).rev() {
                    let offset = 8 * i;
                    let end_idx = self.melspectrogram_buffer.len() - offset;
                    let start_idx = if end_idx >= 76 { end_idx - 76 } else { 0 };

                    if (end_idx - start_idx) == 76 {
                        let mut melspec_window = Vec::with_capacity(76 * features_per_frame);
                        let buffer_vec: Vec<_> = self.melspectrogram_buffer.iter().collect();
                        for frame_idx in start_idx..end_idx {
                            melspec_window.extend(buffer_vec[frame_idx]);
                        }

                        if melspec_window.len() == 76 * 32 {
                            let embedding =
                                self._get_embeddings_from_melspec(&melspec_window)?;
                            self.feature_buffer.push_back(embedding);

                            if self.feature_buffer.len() > self.feature_buffer_max_len {
                                self.feature_buffer.pop_front();
                            }
                        }
                    }
                }
            }

            processed_samples = self.accumulated_samples;
            self.accumulated_samples = 0;
        }

        // Match Python: return processed_samples if processed_samples != 0 else self.accumulated_samples
        Ok(if processed_samples != 0 {
            processed_samples
        } else {
            self.accumulated_samples
        })
    }
}
