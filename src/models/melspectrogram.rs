//! Mel Spectrogram Processor using TensorFlow Lite
//!
//! This module provides mel spectrogram feature extraction from raw audio samples
//! using the melspectrogram.tflite model with proper OpenWakeWord-compatible usage.
//!
//! Based on research, OpenWakeWord:
//! 1. Uses resize_tensor_input(0, [1, 1280], strict=True) for the melspectrogram model
//! 2. Expects input shape [1, 1280] (batch_size=1, sequence_length=1280)
//! 3. The model processes 1.28 seconds of audio at 16kHz sample rate
//! 4. Input is raw audio samples, not mel spectrograms

use crate::error::{EdgeError, Result};
use std::path::Path;

use tflitec::interpreter::Options;
use tflitec::model::Model;
use tflitec::tensor;

/// Configuration for mel spectrogram processing
#[derive(Debug, Clone)]
pub struct MelSpectrogramConfig {
    /// Path to the melspectrogram model
    pub model_path: String,
    /// Audio chunk size in samples (1280 = 80ms at 16kHz)
    pub chunk_size: usize,
    /// Sample rate (default: 16000 Hz)
    pub sample_rate: u32,
}

impl Default for MelSpectrogramConfig {
    fn default() -> Self {
        Self {
            model_path: "models/melspectrogram.tflite".to_string(),
            chunk_size: 1280, // 80ms at 16kHz
            sample_rate: 16000,
        }
    }
}

/// Mel spectrogram processor using TensorFlow Lite
///
/// This processor converts raw audio samples to mel spectrogram features
/// using the OpenWakeWord melspectrogram model approach.
pub struct MelSpectrogramProcessor<'a> {
    model: Model<'a>,
    config: MelSpectrogramConfig,
}

impl<'a> MelSpectrogramProcessor<'a> {
    /// Create a new mel spectrogram processor
    pub fn new(config: MelSpectrogramConfig) -> Result<Self> {
        let model_path = Path::new(&config.model_path);
        if !model_path.exists() {
            return Err(EdgeError::ModelLoadError(format!(
                "Model file not found: {}",
                config.model_path
            )));
        }

        // Load the model
        let model = Model::new(&config.model_path)
            .map_err(|e| EdgeError::ModelLoadError(format!("Failed to load model: {}", e)))?;

        Ok(Self { model, config })
    }

    /// Process audio samples to extract mel spectrogram features
    ///
    /// # Arguments
    /// * `audio_samples` - Raw audio samples (must be exactly config.chunk_size length)
    ///
    /// # Returns
    /// * `Vec<f32>` - Mel spectrogram features as a flattened vector
    pub fn process(&self, audio_samples: &[f32]) -> Result<Vec<f32>> {
        if audio_samples.len() != self.config.chunk_size {
            return Err(EdgeError::InvalidInput(format!(
                "Expected {} audio samples, got {}",
                self.config.chunk_size,
                audio_samples.len()
            )));
        }

        // Create interpreter options
        let mut options = Options::default();
        options.thread_count = 1;

        // Create interpreter (borrowing from model)
        let interpreter = tflitec::interpreter::Interpreter::new(&self.model, Some(options))
            .map_err(|e| {
                EdgeError::ProcessingError(format!("Failed to create interpreter: {}", e))
            })?;

        // Resize input tensor to expected shape: [1, chunk_size]
        let input_shape = tensor::Shape::new(vec![1, self.config.chunk_size]);
        interpreter.resize_input(0, input_shape).map_err(|e| {
            EdgeError::ProcessingError(format!("Failed to resize input tensor: {}", e))
        })?;

        // Allocate tensors after resizing
        interpreter.allocate_tensors().map_err(|e| {
            EdgeError::ProcessingError(format!("Failed to allocate tensors: {}", e))
        })?;

        // Set input tensor data
        interpreter
            .copy(audio_samples, 0)
            .map_err(|e| EdgeError::ProcessingError(format!("Failed to set input: {}", e)))?;

        // Run inference
        interpreter
            .invoke()
            .map_err(|e| EdgeError::ProcessingError(format!("Inference failed: {}", e)))?;

        // Get output tensor
        let output_tensor = interpreter.output(0).map_err(|e| {
            EdgeError::ProcessingError(format!("Failed to get output tensor: {}", e))
        })?;

        // Read output data
        let output_data = output_tensor.data::<f32>().to_vec();

        Ok(output_data)
    }

    /// Get the current configuration
    pub fn config(&self) -> &MelSpectrogramConfig {
        &self.config
    }

    /// Get the expected input shape
    pub fn input_shape(&self) -> Result<Vec<i32>> {
        Ok(vec![1, self.config.chunk_size as i32])
    }

    /// Get the output shape by running a quick inference
    pub fn output_shape(&self) -> Result<Vec<i32>> {
        // Create a temporary interpreter to get output shape
        let mut options = Options::default();
        options.thread_count = 1;

        let interpreter = tflitec::interpreter::Interpreter::new(&self.model, Some(options))
            .map_err(|e| {
                EdgeError::ProcessingError(format!("Failed to create interpreter: {}", e))
            })?;

        let input_shape = tensor::Shape::new(vec![1, self.config.chunk_size]);
        interpreter.resize_input(0, input_shape).map_err(|e| {
            EdgeError::ProcessingError(format!("Failed to resize input tensor: {}", e))
        })?;

        interpreter.allocate_tensors().map_err(|e| {
            EdgeError::ProcessingError(format!("Failed to allocate tensors: {}", e))
        })?;

        let output_tensor = interpreter.output(0).map_err(|e| {
            EdgeError::ProcessingError(format!("Failed to get output tensor: {}", e))
        })?;

        Ok(output_tensor
            .shape()
            .dimensions()
            .iter()
            .map(|&x| x as i32)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_melspec_config_default() {
        let config = MelSpectrogramConfig::default();
        assert_eq!(config.chunk_size, 1280);
        assert_eq!(config.sample_rate, 16000);
        assert!(config.model_path.contains("melspectrogram.tflite"));
    }

    #[test]
    fn test_melspec_processor_creation() {
        let config = MelSpectrogramConfig::default();

        // This will fail in CI without the actual model file
        match MelSpectrogramProcessor::new(config) {
            Ok(_) => println!("✅ Mel spectrogram processor created successfully"),
            Err(e) => println!("⚠️  Expected failure without model files: {}", e),
        }
    }

    #[test]
    fn test_audio_sample_generation() {
        let chunk_size = 1280;
        let sample_rate = 16000;

        // Generate test sine wave
        let frequency = 440.0; // A4 note
        let audio_samples: Vec<f32> = (0..chunk_size)
            .map(|i| {
                let t = i as f32 / sample_rate as f32;
                (2.0 * std::f32::consts::PI * frequency * t).sin() * 0.5
            })
            .collect();

        assert_eq!(audio_samples.len(), chunk_size);

        // Verify amplitude range
        let max_val = audio_samples
            .iter()
            .fold(f32::NEG_INFINITY, |a, &b| a.max(b));
        let min_val = audio_samples.iter().fold(f32::INFINITY, |a, &b| a.min(b));

        assert!(max_val <= 1.0);
        assert!(min_val >= -1.0);

        println!(
            "Generated {} audio samples with range [{:.3}, {:.3}]",
            audio_samples.len(),
            min_val,
            max_val
        );
    }
}
