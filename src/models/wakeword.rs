//! Wakeword Detection using TensorFlow Lite
//!
//! This module provides wakeword detection capabilities using the hey_mycroft model
//! with mel spectrogram feature preprocessing.

use crate::error::{EdgeError, Result};
use std::sync::{Mutex, OnceLock};

use tflitec::interpreter::{Interpreter, Options};
use tflitec::model::Model;
use tflitec::tensor::Shape;

// Static storage for the wakeword model and interpreter
static WAKEWORD_MODEL: OnceLock<Model<'static>> = OnceLock::new();
static WAKEWORD_INTERPRETER: OnceLock<Mutex<Interpreter<'static>>> = OnceLock::new();
static WAKEWORD_CONFIG: OnceLock<WakewordModelConfig> = OnceLock::new();

/// Configuration for wakeword model
#[derive(Debug, Clone)]
struct WakewordModelConfig {
    expected_input_size: usize,
}

/// Wakeword detection result
#[derive(Debug, Clone)]
pub struct WakewordDetection {
    /// Whether a wakeword was detected
    pub detected: bool,
    /// Confidence score (0.0 - 1.0)
    pub confidence: f32,
    /// Timestamp when detection occurred
    pub timestamp: std::time::Instant,
}

/// Simple wrapper for wakeword model used by the detection pipeline  
pub struct WakewordModel;

impl WakewordModel {
    pub fn new(model_path: &str) -> Result<Self> {
        // Initialize the static model
        let model = Model::new(model_path).map_err(|e| {
            EdgeError::ModelLoadError(format!("Failed to load wakeword model: {}", e))
        })?;

        let model = WAKEWORD_MODEL.get_or_init(|| model);

        // Corrected: Model expects [1, 16, 96] = 1536 features (16 embedding frames × 96 features each)
        let expected_input_size = 1536;

        // Store the config
        let config = WakewordModelConfig {
            expected_input_size,
        };

        WAKEWORD_CONFIG
            .set(config)
            .map_err(|_| EdgeError::ModelLoadError("Failed to set wakeword config".to_string()))?;

        // Create interpreter options
        let mut options = Options::default();
        options.thread_count = 1;

        // Create the static interpreter
        let interpreter = Interpreter::new(model, Some(options)).map_err(|e| {
            EdgeError::ModelLoadError(format!("Failed to create wakeword interpreter: {}", e))
        })?;

        // Resize input tensor to [1, expected_input_size]
        let input_shape = Shape::new(vec![1, expected_input_size]);
        interpreter.resize_input(0, input_shape).map_err(|e| {
            EdgeError::ModelLoadError(format!("Failed to resize wakeword input: {}", e))
        })?;

        // Allocate tensors
        interpreter.allocate_tensors().map_err(|e| {
            EdgeError::ModelLoadError(format!("Failed to allocate wakeword tensors: {}", e))
        })?;

        // Store the interpreter in a mutex for thread safety
        WAKEWORD_INTERPRETER
            .set(Mutex::new(interpreter))
            .map_err(|_| {
                EdgeError::ModelLoadError("Failed to initialize wakeword interpreter".to_string())
            })?;

        Ok(Self)
    }

    pub fn predict(&self, features: &[f32]) -> Result<f32> {
        let config = WAKEWORD_CONFIG.get().ok_or_else(|| {
            EdgeError::ProcessingError("Wakeword model not initialized".to_string())
        })?;

        if features.len() != config.expected_input_size {
            return Err(EdgeError::InvalidInput(format!(
                "Expected {} features, got {}",
                config.expected_input_size,
                features.len()
            )));
        }

        // Get the static interpreter
        let interpreter_mutex = WAKEWORD_INTERPRETER.get().ok_or_else(|| {
            EdgeError::ProcessingError("Wakeword model not initialized".to_string())
        })?;

        let interpreter = interpreter_mutex.lock().map_err(|e| {
            EdgeError::ProcessingError(format!("Failed to lock interpreter: {}", e))
        })?;

        // Set input tensor data (use original features without normalization)
        interpreter.copy(features, 0).map_err(|e| {
            EdgeError::ProcessingError(format!("Failed to set wakeword input: {}", e))
        })?;

        // Run inference
        interpreter
            .invoke()
            .map_err(|e| EdgeError::ProcessingError(format!("Wakeword inference failed: {}", e)))?;

        // Get output - model outputs [4, 1] so we need to check the shape
        let output_tensor = interpreter.output(0).map_err(|e| {
            EdgeError::ProcessingError(format!("Failed to get wakeword output: {}", e))
        })?;

        let output_data = output_tensor.data::<f32>();
        if output_data.is_empty() {
            return Err(EdgeError::ProcessingError(
                "Empty wakeword output".to_string(),
            ));
        }

        if output_data.len() != 1 {
            log::warn!("Expected 1 output value, got {}", output_data.len());
            return Ok(0.0);
        }

        // The single output value is already a confidence score
        let confidence = output_data[0];

        // Validate confidence is in expected range and warn if not
        if confidence < 0.0 || confidence > 1.0 {
            log::warn!(
                "Wakeword model output out of range: {:.6} (expected 0.0-1.0). This may indicate model or preprocessing issues.",
                confidence
            );
        }

        // Return the raw confidence score
        Ok(confidence)
    }
}
