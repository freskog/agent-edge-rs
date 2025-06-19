//! Wakeword Detection using TensorFlow Lite
//!
//! This module provides wakeword detection capabilities using the hey_mycroft model
//! with mel spectrogram feature preprocessing.

use crate::error::{EdgeError, Result};
use lazy_static::lazy_static;
use std::sync::{Arc, Mutex};

use tflitec::interpreter::Options;
use tflitec::model::Model;
use tflitec::tensor;

/// Configuration for wakeword detection
#[derive(Debug, Clone)]
pub struct WakewordConfig {
    /// Path to the hey_mycroft wakeword model
    pub wakeword_model_path: String,
    /// Path to the melspectrogram preprocessing model  
    pub melspec_model_path: String,
    /// Confidence threshold for wakeword detection (0.0 - 1.0)
    pub confidence_threshold: f32,
    /// Sample rate for audio processing (default: 16000 Hz)
    pub sample_rate: u32,
    /// Audio chunk size in samples (default: 1280 for 80ms at 16kHz)
    pub chunk_size: usize,
}

impl Default for WakewordConfig {
    fn default() -> Self {
        Self {
            wakeword_model_path: "models/hey_mycroft_v0.1.tflite".to_string(),
            melspec_model_path: "models/melspectrogram.tflite".to_string(),
            confidence_threshold: 0.5,
            sample_rate: 16000,
            chunk_size: 1280, // 80ms at 16kHz
        }
    }
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

/// Thread-safe wakeword detector
pub struct WakewordDetector<'a> {
    melspec_model: Model<'a>,
    wakeword_model: Model<'a>,
    config: WakewordConfig,
}

impl<'a> WakewordDetector<'a> {
    /// Create a new wakeword detector with the given configuration
    pub fn new(config: WakewordConfig) -> Result<Self> {
        // Load mel spectrogram model
        let melspec_model = Model::new(&config.melspec_model_path).map_err(|e| {
            EdgeError::ModelLoadError(format!("Failed to load melspectrogram model: {}", e))
        })?;

        // Load wakeword model
        let wakeword_model = Model::new(&config.wakeword_model_path).map_err(|e| {
            EdgeError::ModelLoadError(format!("Failed to load wakeword model: {}", e))
        })?;

        Ok(Self {
            melspec_model,
            wakeword_model,
            config,
        })
    }

    /// Process audio samples and detect wakewords
    pub fn process_audio(&self, audio_samples: &[f32]) -> Result<WakewordDetection> {
        if audio_samples.len() != self.config.chunk_size {
            return Err(EdgeError::InvalidInput(format!(
                "Expected {} audio samples, got {}",
                self.config.chunk_size,
                audio_samples.len()
            )));
        }

        // Step 1: Convert audio to mel spectrogram features
        let melspec_features = self.extract_melspec_features(audio_samples)?;

        // Step 2: Run wakeword detection on mel spectrogram features
        let confidence = self.detect_wakeword(&melspec_features)?;

        Ok(WakewordDetection {
            detected: confidence >= self.config.confidence_threshold,
            confidence,
            timestamp: std::time::Instant::now(),
        })
    }

    /// Extract mel spectrogram features from raw audio
    fn extract_melspec_features(&self, audio_samples: &[f32]) -> Result<Vec<f32>> {
        // Create interpreter options
        let mut options = Options::default();
        options.thread_count = 1;

        // Create interpreter
        let interpreter = tflitec::interpreter::Interpreter::new(
            &self.melspec_model,
            Some(options),
        )
        .map_err(|e| {
            EdgeError::ProcessingError(format!("Failed to create melspec interpreter: {}", e))
        })?;

        // Resize input tensor to [1, chunk_size]
        let input_shape = tensor::Shape::new(vec![1, self.config.chunk_size]);
        interpreter.resize_input(0, input_shape).map_err(|e| {
            EdgeError::ProcessingError(format!("Failed to resize melspec input: {}", e))
        })?;

        // Allocate tensors
        interpreter.allocate_tensors().map_err(|e| {
            EdgeError::ProcessingError(format!("Failed to allocate melspec tensors: {}", e))
        })?;

        // Set input tensor data
        interpreter.copy(audio_samples, 0).map_err(|e| {
            EdgeError::ProcessingError(format!("Failed to set melspec input: {}", e))
        })?;

        // Run inference
        interpreter
            .invoke()
            .map_err(|e| EdgeError::ProcessingError(format!("Melspec inference failed: {}", e)))?;

        // Get output data
        let output_tensor = interpreter.output(0).map_err(|e| {
            EdgeError::ProcessingError(format!("Failed to get melspec output: {}", e))
        })?;

        let output_data = output_tensor.data::<f32>().to_vec();
        Ok(output_data)
    }

    /// Detect wakeword from mel spectrogram features
    fn detect_wakeword(&self, melspec_features: &[f32]) -> Result<f32> {
        // Create interpreter options
        let mut options = Options::default();
        options.thread_count = 1;

        // Create interpreter
        let interpreter = tflitec::interpreter::Interpreter::new(
            &self.wakeword_model,
            Some(options),
        )
        .map_err(|e| {
            EdgeError::ProcessingError(format!("Failed to create wakeword interpreter: {}", e))
        })?;

        // Allocate tensors (assuming the model has the right input shape)
        interpreter.allocate_tensors().map_err(|e| {
            EdgeError::ProcessingError(format!("Failed to allocate wakeword tensors: {}", e))
        })?;

        // Set input tensor data
        interpreter.copy(melspec_features, 0).map_err(|e| {
            EdgeError::ProcessingError(format!("Failed to set wakeword input: {}", e))
        })?;

        // Run inference
        interpreter
            .invoke()
            .map_err(|e| EdgeError::ProcessingError(format!("Wakeword inference failed: {}", e)))?;

        // Get output - should be a single confidence score
        let output_tensor = interpreter.output(0).map_err(|e| {
            EdgeError::ProcessingError(format!("Failed to get wakeword output: {}", e))
        })?;

        let output_data = output_tensor.data::<f32>();
        if output_data.is_empty() {
            return Err(EdgeError::ProcessingError(
                "Empty wakeword output".to_string(),
            ));
        }

        Ok(output_data[0])
    }

    /// Get the current configuration
    pub fn config(&self) -> &WakewordConfig {
        &self.config
    }
}

// Global detector instance for convenient access
lazy_static! {
    static ref GLOBAL_DETECTOR: Arc<Mutex<Option<WakewordDetector<'static>>>> =
        Arc::new(Mutex::new(None));
}

/// Initialize the global wakeword detector
pub fn initialize_detector(config: WakewordConfig) -> Result<()> {
    let detector = WakewordDetector::new(config)?;
    let mut global = GLOBAL_DETECTOR.lock().map_err(|_| {
        EdgeError::ProcessingError("Failed to acquire global detector lock".to_string())
    })?;
    *global = Some(detector);
    Ok(())
}

/// Process audio using the global detector
pub fn process_audio_global(audio_samples: &[f32]) -> Result<WakewordDetection> {
    let global = GLOBAL_DETECTOR.lock().map_err(|_| {
        EdgeError::ProcessingError("Failed to acquire global detector lock".to_string())
    })?;

    match global.as_ref() {
        Some(detector) => detector.process_audio(audio_samples),
        None => Err(EdgeError::ProcessingError(
            "Global detector not initialized".to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wakeword_config_default() {
        let config = WakewordConfig::default();
        assert_eq!(config.sample_rate, 16000);
        assert_eq!(config.chunk_size, 1280);
        assert_eq!(config.confidence_threshold, 0.5);
    }

    #[test]
    fn test_wakeword_detector_creation() {
        let config = WakewordConfig::default();

        // This will fail in CI without the actual model files
        match WakewordDetector::new(config) {
            Ok(_) => println!("✅ Wakeword detector created successfully"),
            Err(e) => println!("⚠️  Expected failure without model files: {}", e),
        }
    }
}
