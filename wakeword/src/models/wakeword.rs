//! Wakeword Detection using TensorFlow Lite
//!
//! This module provides wakeword detection capabilities using the hey_mycroft model
//! with mel spectrogram feature preprocessing.

use crate::error::{EdgeError, Result};

use tflitec::interpreter::{Interpreter, Options};
use tflitec::model::Model;
use tflitec::tensor::Shape;

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
pub struct WakewordModel {
    interpreter: Interpreter<'static>,
    expected_input_size: usize,
}

impl WakewordModel {
    pub fn new(model_path: &str) -> Result<Self> {
        // Load the model and leak it for 'static lifetime
        let model = Box::leak(Box::new(Model::new(model_path).map_err(|e| {
            EdgeError::ModelLoadError(format!("Failed to load wakeword model: {}", e))
        })?));

        // Corrected: Model expects [1, 16, 96] = 1536 features (16 embedding frames × 96 features each)
        let expected_input_size = 1536;

        // Create interpreter options - start with XNNPACK enabled (if compiled in)
        let mut options = Options::default();
        options.thread_count = 1;
        // XNNPACK is now enabled with pthreadpool linking

        // Try to create the interpreter; if it fails (e.g. because XNNPACK was
        // built with unsupported CPU instructions such as `sdot` on Cortex-A53)
        // fallback to a plain interpreter without the delegate so that the
        // application can still run – albeit a bit slower.
        let interpreter = match Interpreter::new(model, Some(options)) {
            Ok(i) => i,
            Err(e) => {
                log::warn!(
                    "Failed to create wakeword interpreter with XNNPACK: {e}. Falling back to interpreter without XNNPACK."
                );

                let mut fallback = Options::default();
                fallback.thread_count = 1;
                // XNNPACK is now enabled with pthreadpool linking

                Interpreter::new(model, Some(fallback)).map_err(|e2| {
                    EdgeError::ModelLoadError(format!(
                        "Failed to create wakeword interpreter without XNNPACK after first attempt failed: {e2}"
                    ))
                })?
            }
        };

        // Resize input tensor to [1, expected_input_size]
        let input_shape = Shape::new(vec![1, expected_input_size]);
        interpreter.resize_input(0, input_shape).map_err(|e| {
            EdgeError::ModelLoadError(format!("Failed to resize wakeword input: {}", e))
        })?;

        // Allocate tensors
        interpreter.allocate_tensors().map_err(|e| {
            EdgeError::ModelLoadError(format!("Failed to allocate wakeword tensors: {}", e))
        })?;

        Ok(Self {
            interpreter,
            expected_input_size,
        })
    }

    pub fn predict(&mut self, features: &[f32]) -> Result<f32> {
        if features.len() != self.expected_input_size {
            return Err(EdgeError::InvalidInput(format!(
                "Expected {} features, got {}",
                self.expected_input_size,
                features.len()
            )));
        }

        // Set input tensor data (use original features without normalization)
        self.interpreter.copy(features, 0).map_err(|e| {
            EdgeError::ProcessingError(format!("Failed to set wakeword input: {}", e))
        })?;

        // Run inference
        self.interpreter
            .invoke()
            .map_err(|e| EdgeError::ProcessingError(format!("Wakeword inference failed: {}", e)))?;

        // Get output - model outputs [4, 1] so we need to check the shape
        let output_tensor = self.interpreter.output(0).map_err(|e| {
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
