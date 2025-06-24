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
use std::sync::{Mutex, OnceLock};

use tflitec::interpreter::{Interpreter, Options};
use tflitec::model::Model;
use tflitec::tensor::Shape;

// Static storage for the model and interpreter
static MELSPEC_MODEL: OnceLock<Model<'static>> = OnceLock::new();
static MELSPEC_INTERPRETER: OnceLock<Mutex<Interpreter<'static>>> = OnceLock::new();
static MELSPEC_CONFIG: OnceLock<MelSpectrogramConfig> = OnceLock::new();

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

/// Mel spectrogram model used by the detection pipeline
///
/// This model converts raw audio samples to mel spectrogram features
/// using the OpenWakeWord melspectrogram model approach.
pub struct MelSpectrogramModel;

impl MelSpectrogramModel {
    /// Create a new mel spectrogram model
    pub fn new(model_path: &str) -> Result<Self> {
        let config = MelSpectrogramConfig {
            model_path: model_path.to_string(),
            ..Default::default()
        };

        let model_path = Path::new(&config.model_path);
        if !model_path.exists() {
            return Err(EdgeError::ModelLoadError(format!(
                "Model file not found: {}",
                config.model_path
            )));
        }

        // Initialize the static model
        let model = Model::new(&config.model_path)
            .map_err(|e| EdgeError::ModelLoadError(format!("Failed to load model: {}", e)))?;

        let model = MELSPEC_MODEL.get_or_init(|| model);

        // Store the config
        MELSPEC_CONFIG
            .set(config.clone())
            .map_err(|_| EdgeError::ModelLoadError("Failed to set config".to_string()))?;

        // Create interpreter options
        let mut options = Options::default();
        options.thread_count = 1;

        // Create the static interpreter
        let interpreter = Interpreter::new(model, Some(options)).map_err(|e| {
            EdgeError::ModelLoadError(format!("Failed to create interpreter: {}", e))
        })?;

        // Resize input tensor to expected shape: [1, chunk_size]
        let input_shape = Shape::new(vec![1, config.chunk_size]);
        interpreter.resize_input(0, input_shape).map_err(|e| {
            EdgeError::ModelLoadError(format!("Failed to resize input tensor: {}", e))
        })?;

        // Allocate tensors after resizing
        interpreter
            .allocate_tensors()
            .map_err(|e| EdgeError::ModelLoadError(format!("Failed to allocate tensors: {}", e)))?;

        // Log the output shape for debugging
        let output_tensor = interpreter.output(0).map_err(|e| {
            EdgeError::ModelLoadError(format!("Failed to get melspec output tensor: {}", e))
        })?;
        let output_shape = output_tensor.shape();
        let output_size = output_shape.dimensions().iter().product::<usize>();
        log::info!(
            "Melspectrogram model output shape: {:?} (size: {})",
            output_shape.dimensions(),
            output_size
        );

        // Store the interpreter in a mutex for thread safety
        MELSPEC_INTERPRETER
            .set(Mutex::new(interpreter))
            .map_err(|_| {
                EdgeError::ModelLoadError("Failed to initialize interpreter".to_string())
            })?;

        Ok(Self)
    }

    /// Process audio samples to extract mel spectrogram features
    ///
    /// # Arguments
    /// * `audio_samples` - Raw audio samples (must be exactly config.chunk_size length)
    ///
    /// # Returns
    /// * `Vec<f32>` - Mel spectrogram features as a flattened vector
    pub fn predict(&self, audio_samples: &[f32]) -> Result<Vec<f32>> {
        let config = MELSPEC_CONFIG.get().ok_or_else(|| {
            EdgeError::ProcessingError("MelSpectrogram model not initialized".to_string())
        })?;

        if audio_samples.len() != config.chunk_size {
            return Err(EdgeError::InvalidInput(format!(
                "Expected {} audio samples, got {}",
                config.chunk_size,
                audio_samples.len()
            )));
        }

        // Get the static interpreter
        let interpreter_mutex = MELSPEC_INTERPRETER.get().ok_or_else(|| {
            EdgeError::ProcessingError("MelSpectrogram model not initialized".to_string())
        })?;

        let interpreter = interpreter_mutex.lock().map_err(|e| {
            EdgeError::ProcessingError(format!("Failed to lock interpreter: {}", e))
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

        // Apply OpenWakeWord's melspectrogram transform: x/10 + 2
        let transformed_data: Vec<f32> = output_data.iter().map(|&x| x / 10.0 + 2.0).collect();

        log::debug!(
            "Melspectrogram model produced {} features",
            transformed_data.len()
        );

        Ok(transformed_data)
    }
}

/// Type alias for backwards compatibility
pub type MelspectrogramModel = MelSpectrogramModel;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_melspec_config_default() {
        let config = MelSpectrogramConfig::default();
        assert_eq!(config.model_path, "models/melspectrogram.tflite");
        assert_eq!(config.chunk_size, 1280);
        assert_eq!(config.sample_rate, 16000);
    }

    #[test]
    fn test_melspec_model_creation() {
        let result = MelSpectrogramModel::new("non_existent_model.tflite");
        assert!(result.is_err());
    }

    #[test]
    fn test_audio_sample_generation() {
        // Test generating audio samples
        let sample_rate = 16000;
        let duration_ms = 80; // 80ms
        let chunk_size = (sample_rate * duration_ms) / 1000;

        assert_eq!(chunk_size, 1280);

        // Generate some dummy audio samples
        let audio_samples: Vec<f32> = (0..chunk_size).map(|i| (i as f32 * 0.001).sin()).collect();

        assert_eq!(audio_samples.len(), 1280);
        assert!(audio_samples.iter().all(|&x| x >= -1.0 && x <= 1.0));
    }
}
