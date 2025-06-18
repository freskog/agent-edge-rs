use crate::error::{EdgeError, Result};
use std::collections::VecDeque;
use tflite::ops::builtin::BuiltinOpResolver;
use tflite::{FlatBufferModel, InterpreterBuilder};

/// Embedded wakeword detection model (baked into binary for edge deployment)
const WAKEWORD_MODEL_BYTES: &[u8] = include_bytes!("../../models/hey_mycroft_v0.1.tflite");

/// Configuration for wakeword detection
#[derive(Debug, Clone)]
pub struct WakewordConfig {
    /// Number of mel spectrogram frames to accumulate before detection
    pub frame_window_size: usize,
    /// Confidence threshold for wakeword detection (0.0 to 1.0)
    pub confidence_threshold: f32,
    /// Frame shift - how many frames to advance the window each time
    pub frame_shift: usize,
}

impl Default for WakewordConfig {
    fn default() -> Self {
        Self {
            frame_window_size: 76,     // Typical window size for OpenWakeWord models
            confidence_threshold: 0.5, // OpenWakeWord default threshold
            frame_shift: 1,            // Process every new frame
        }
    }
}

/// Result of wakeword detection
#[derive(Debug, Clone)]
pub struct WakewordDetection {
    /// Confidence score (0.0 to 1.0)
    pub confidence: f32,
    /// Whether the detection exceeds the threshold
    pub detected: bool,
    /// Timestamp of detection (frame number)
    pub frame_number: u64,
}

/// Wakeword detection processor that analyzes accumulated mel spectrogram frames
pub struct WakewordDetector {
    config: WakewordConfig,
    interpreter: tflite::Interpreter,
    frame_buffer: VecDeque<Vec<f32>>,
    frame_count: u64,
    frame_feature_size: usize,
    input_index: i32,
    output_index: i32,
}

impl WakewordDetector {
    /// Create a new wakeword detector with embedded model
    pub fn new(config: WakewordConfig, mel_feature_size: usize) -> Result<Self> {
        log::info!(
            "Loading embedded wakeword model ({} bytes)",
            WAKEWORD_MODEL_BYTES.len()
        );

        // Load the embedded model
        let model = FlatBufferModel::build_from_buffer(WAKEWORD_MODEL_BYTES.to_vec())
            .map_err(|e| EdgeError::Model(format!("Failed to load wakeword model: {}", e)))?;

        // Create resolver and interpreter builder
        let resolver = BuiltinOpResolver::default();
        let builder = InterpreterBuilder::new(&model, &resolver).map_err(|e| {
            EdgeError::Model(format!("Failed to create interpreter builder: {}", e))
        })?;

        // Build interpreter
        let mut interpreter = builder
            .build()
            .map_err(|e| EdgeError::Model(format!("Failed to build interpreter: {}", e)))?;

        // Allocate tensors
        interpreter
            .allocate_tensors()
            .map_err(|e| EdgeError::Model(format!("Failed to allocate tensors: {}", e)))?;

        // Get input and output indices
        let inputs = interpreter.inputs().to_vec();
        let outputs = interpreter.outputs().to_vec();

        if inputs.is_empty() || outputs.is_empty() {
            return Err(EdgeError::Model(
                "Model has no inputs or outputs".to_string(),
            ));
        }

        let input_index = inputs[0];
        let output_index = outputs[0];

        // Get tensor info for validation and logging
        let input_tensor = interpreter
            .tensor_info(input_index)
            .ok_or_else(|| EdgeError::Model("Failed to get input tensor info".to_string()))?;
        let output_tensor = interpreter
            .tensor_info(output_index)
            .ok_or_else(|| EdgeError::Model("Failed to get output tensor info".to_string()))?;

        log::info!("Wakeword detector initialized:");
        log::info!("  - Frame window size: {}", config.frame_window_size);
        log::info!(
            "  - Confidence threshold: {:.2}",
            config.confidence_threshold
        );
        log::info!("  - Frame shift: {}", config.frame_shift);
        log::info!("  - Input tensor shape: {:?}", input_tensor.dims);
        log::info!("  - Output tensor shape: {:?}", output_tensor.dims);

        // Validate mel feature size matches expected input
        let expected_feature_size = if input_tensor.dims.len() >= 2 {
            input_tensor.dims[input_tensor.dims.len() - 1] as usize // Last dimension is typically feature size
        } else {
            return Err(EdgeError::Model("Invalid input tensor shape".to_string()));
        };

        if mel_feature_size != expected_feature_size {
            log::warn!(
                "Mel feature size {} doesn't match expected input size {}",
                mel_feature_size,
                expected_feature_size
            );
        }

        let frame_window_size = config.frame_window_size;
        Ok(Self {
            config,
            interpreter,
            frame_buffer: VecDeque::with_capacity(frame_window_size),
            frame_count: 0,
            frame_feature_size: mel_feature_size,
            input_index,
            output_index,
        })
    }

    /// Add a new mel spectrogram frame and check for wakeword detection
    pub fn process_frame(&mut self, mel_features: Vec<f32>) -> Result<Option<WakewordDetection>> {
        // Validate feature size
        if mel_features.len() != self.frame_feature_size {
            return Err(EdgeError::Model(format!(
                "Invalid feature size: expected {}, got {}",
                self.frame_feature_size,
                mel_features.len()
            )));
        }

        // Add new frame to buffer
        self.frame_buffer.push_back(mel_features);
        self.frame_count += 1;

        // Remove old frames if buffer is full
        while self.frame_buffer.len() > self.config.frame_window_size {
            self.frame_buffer.pop_front();
        }

        // Only run detection when we have enough frames
        if self.frame_buffer.len() < self.config.frame_window_size {
            return Ok(None);
        }

        // Prepare input tensor data by flattening frame buffer
        let mut input_data = Vec::new();
        for frame in &self.frame_buffer {
            input_data.extend_from_slice(frame);
        }

        // Get input tensor data and copy flattened data
        let tensor_input_data: &mut [f32] = self
            .interpreter
            .tensor_data_mut(self.input_index)
            .map_err(|e| EdgeError::Model(format!("Failed to get input tensor data: {}", e)))?;

        if tensor_input_data.len() < input_data.len() {
            return Err(EdgeError::Model(
                "Input tensor too small for frame data".to_string(),
            ));
        }

        tensor_input_data[..input_data.len()].copy_from_slice(&input_data);

        // Run inference
        self.interpreter
            .invoke()
            .map_err(|e| EdgeError::Model(format!("Inference failed: {}", e)))?;

        // Get output tensor data
        let output_data: &[f32] = self
            .interpreter
            .tensor_data(self.output_index)
            .map_err(|e| EdgeError::Model(format!("Failed to get output tensor data: {}", e)))?;

        // Assume single output value for binary classification
        let confidence = if output_data.is_empty() {
            return Err(EdgeError::Model("Empty output tensor".to_string()));
        } else {
            output_data[0] // For binary classification, often just one output value
        };

        // Create detection result
        let detected = confidence >= self.config.confidence_threshold;
        let detection = WakewordDetection {
            confidence,
            detected,
            frame_number: self.frame_count,
        };

        if detected {
            log::info!(
                "Wakeword detected! Confidence: {:.3}, Frame: {}",
                confidence,
                self.frame_count
            );
        }

        Ok(Some(detection))
    }

    /// Get current frame buffer status (current_size, capacity)
    pub fn buffer_status(&self) -> (usize, usize) {
        (self.frame_buffer.len(), self.config.frame_window_size)
    }

    /// Reset the frame buffer (useful for testing or state reset)
    pub fn reset_buffer(&mut self) {
        self.frame_buffer.clear();
        self.frame_count = 0;
    }

    /// Update the confidence threshold
    pub fn set_threshold(&mut self, threshold: f32) {
        if threshold >= 0.0 && threshold <= 1.0 {
            self.config.confidence_threshold = threshold;
            log::info!("Updated confidence threshold to {:.2}", threshold);
        } else {
            log::warn!(
                "Invalid threshold {:.2}, must be between 0.0 and 1.0",
                threshold
            );
        }
    }

    /// Get current confidence threshold
    pub fn threshold(&self) -> f32 {
        self.config.confidence_threshold
    }
}
