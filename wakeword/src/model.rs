//! OpenWakeWord Model implementation
//!
//! This module provides the main Model class that matches the Python implementation,
//! including proper prediction buffer management and simplified prediction interface.

use std::collections::{HashMap, VecDeque};
use tflitec::interpreter::Interpreter;
use tflitec::model::Model as TfliteModel;

use crate::error::{OpenWakeWordError, Result};
use crate::utils::AudioFeatures;
use crate::{get_model_class_mappings, FEATURE_MODELS, MODELS};

/// Type alias for prediction results
pub type PredictionResult = HashMap<String, f32>;

/// Main model struct that holds all wake word models and shared preprocessor
pub struct Model {
    // Model storage
    models: HashMap<String, Interpreter<'static>>,
    model_inputs: HashMap<String, usize>,

    // Class mappings for multi-class models
    class_mapping: HashMap<String, HashMap<String, String>>,

    // Audio preprocessor (shared)
    preprocessor: AudioFeatures,

    // Prediction buffers (deque with maxlen=30 like Python)
    prediction_buffer: HashMap<String, VecDeque<f32>>,
}

impl Model {
    /// Create a new Model instance
    ///
    /// # Arguments
    /// * `wakeword_models` - List of model names or paths to load
    /// * `class_mapping_dicts` - Optional class mappings for multi-class models
    ///
    /// # Returns
    /// * `Result<Model>` - The created model instance
    pub fn new(
        wakeword_models: Vec<String>,
        class_mapping_dicts: Vec<HashMap<String, String>>,
    ) -> Result<Self> {
        let model_paths = MODELS;
        let mut model_names = Vec::new();
        let mut resolved_paths = Vec::new();

        // Resolve model names to paths
        for model in wakeword_models {
            if std::path::Path::new(&model).exists() {
                // Direct path
                resolved_paths.push(model.clone());
                model_names.push(
                    std::path::Path::new(&model)
                        .file_stem()
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .to_string(),
                );
            } else {
                // Look up by name
                if let Some(path) = model_paths.iter().find(|(name, _)| *name == model) {
                    resolved_paths.push(path.1.to_string());
                    model_names.push(model);
                } else {
                    return Err(OpenWakeWordError::ModelLoadError(format!(
                        "Model not found: {}",
                        model
                    )));
                }
            }
        }

        // Load models
        let mut models = HashMap::new();
        let mut model_inputs = HashMap::new();
        let mut model_outputs = HashMap::new();

        for (model_name, model_path) in model_names.iter().zip(resolved_paths.iter()) {
            log::debug!("Loading model: {} from {}", model_name, model_path);

            // Load TFLite model (same pattern as utils.rs)
            let tflite_model = Box::leak(Box::new(TfliteModel::new(model_path).map_err(|e| {
                OpenWakeWordError::ModelLoadError(format!(
                    "Failed to load model {}: {}",
                    model_name, e
                ))
            })?));

            // Use our XNNPACK-safe interpreter creation
            let interpreter =
                crate::xnnpack_fix::create_interpreter_with_xnnpack_safe(tflite_model, 1).map_err(
                    |e| {
                        OpenWakeWordError::ModelLoadError(format!(
                            "Failed to create interpreter for {}: {}",
                            model_name, e
                        ))
                    },
                )?;

            interpreter.allocate_tensors().map_err(|e| {
                OpenWakeWordError::ModelLoadError(format!(
                    "Failed to allocate tensors for {}: {}",
                    model_name, e
                ))
            })?;

            let input_tensor = interpreter.input(0).map_err(|e| {
                OpenWakeWordError::ModelLoadError(format!(
                    "Failed to get input tensor for {}: {}",
                    model_name, e
                ))
            })?;

            let output_tensor = interpreter.output(0).map_err(|e| {
                OpenWakeWordError::ModelLoadError(format!(
                    "Failed to get output tensor for {}: {}",
                    model_name, e
                ))
            })?;

            // Python uses shape[1] for input_size (number of frames, not total features)
            let input_size = input_tensor
                .shape()
                .dimensions()
                .get(1)
                .copied()
                .unwrap_or(0) as usize;
            let output_size = output_tensor
                .shape()
                .dimensions()
                .get(1)
                .copied()
                .unwrap_or(0) as usize;

            models.insert(model_name.clone(), interpreter);
            model_inputs.insert(model_name.clone(), input_size);
            model_outputs.insert(model_name.clone(), output_size);
        }

        // Set up class mappings
        let mut class_mapping = HashMap::new();
        let default_mappings = get_model_class_mappings();

        for (i, model_name) in model_names.iter().enumerate() {
            if i < class_mapping_dicts.len() {
                class_mapping.insert(model_name.clone(), class_mapping_dicts[i].clone());
            } else if let Some(default_mapping) = default_mappings.get(model_name) {
                class_mapping.insert(model_name.clone(), default_mapping.clone());
            } else {
                // Create default mapping
                let output_size = model_outputs[model_name];
                let mut default_mapping = HashMap::new();
                for j in 0..output_size {
                    default_mapping.insert(j.to_string(), j.to_string());
                }
                class_mapping.insert(model_name.clone(), default_mapping);
            }
        }

        // Initialize shared preprocessor
        let melspec_path = FEATURE_MODELS
            .iter()
            .find(|(name, _)| *name == "melspectrogram")
            .map(|(_, path)| path)
            .unwrap_or(&"models/melspectrogram.tflite");

        let embedding_path = FEATURE_MODELS
            .iter()
            .find(|(name, _)| *name == "embedding")
            .map(|(_, path)| path)
            .unwrap_or(&"models/embedding_model.tflite");

        let preprocessor = AudioFeatures::new(melspec_path, embedding_path, 16000)?;

        // Initialize prediction buffers (deque with maxlen=30)
        let mut prediction_buffer = HashMap::new();
        for model_name in model_names.iter() {
            let deque = VecDeque::with_capacity(30);
            prediction_buffer.insert(model_name.clone(), deque);
        }

        Ok(Model {
            models,
            model_inputs,
            class_mapping,
            preprocessor,
            prediction_buffer,
        })
    }

    /// Reset internal state
    pub fn reset(&mut self) -> Result<()> {
        self.preprocessor.reset()?;
        for buffer in self.prediction_buffer.values_mut() {
            buffer.clear();
        }
        Ok(())
    }

    /// Predict on audio data (matches Python predict method)
    ///
    /// # Arguments
    /// * `x` - Input audio data (16-bit PCM)
    /// * `threshold` - Optional threshold values per model
    /// * `debounce_time` - Time to wait before returning another detection
    ///
    /// # Returns
    /// * `Result<PredictionResult>` - Prediction scores per model
    pub fn predict(
        &mut self,
        x: &[i16],
        threshold: Option<HashMap<String, f32>>,
        debounce_time: f32,
    ) -> Result<PredictionResult> {
        // Process audio in 1280-sample chunks like Python does
        log::debug!("🔍 Starting prediction with {} audio samples", x.len());

        let chunk_size = 1280;
        let mut final_predictions = HashMap::new();

        // Initialize predictions for all models
        let model_names: Vec<String> = self.models.keys().cloned().collect();
        for model_name in &model_names {
            final_predictions.insert(model_name.clone(), 0.0);
        }

        // Process each chunk
        for (chunk_idx, chunk) in x.chunks(chunk_size).enumerate() {
            let mut chunk_vec = chunk.to_vec();

            // Pad chunk to 1280 samples if needed
            while chunk_vec.len() < chunk_size {
                chunk_vec.push(0);
            }

            log::debug!(
                "🔍 Processing chunk {}: {} samples",
                chunk_idx,
                chunk_vec.len()
            );

            // Get audio features for this chunk
            let n_prepared_samples = self.preprocessor.__call__(&chunk_vec)?;
            log::debug!(
                "🔍 Chunk {}: preprocessor returned {} prepared samples",
                chunk_idx,
                n_prepared_samples
            );

            // Process this chunk through each model
            for mdl in &model_names {
                let n_feature_frames = self.model_inputs[mdl];

                // For single chunk processing (1280 samples), we expect to get the right number of features
                if n_prepared_samples == 1280 {
                    let features = self.preprocessor.get_features(n_feature_frames, -1);
                    log::debug!(
                        "🔍 Chunk {}: Model {}: got {} features, expected {}",
                        chunk_idx,
                        mdl,
                        features.len(),
                        n_feature_frames * 96
                    );

                    // Calculate feature stats for comparison with Python
                    if !features.is_empty() {
                        let mean = features.iter().sum::<f32>() / features.len() as f32;
                        let min = features.iter().fold(f32::INFINITY, |a, &b| a.min(b));
                        let max = features.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b));
                        log::debug!(
                            "🔍 Chunk {}: Model {}: feature stats: mean={:.6}, min={:.6}, max={:.6}",
                            chunk_idx, mdl, mean, min, max
                        );
                    }

                    // Only predict if we have enough features (don't pad with zeros)
                    if features.len() >= n_feature_frames * 96 {
                        let prediction = Self::run_model_prediction_static(
                            self.models.get_mut(mdl).unwrap(),
                            &features,
                        )?;

                        log::debug!(
                            "🔍 Chunk {}: Model {}: raw prediction = {}",
                            chunk_idx,
                            mdl,
                            prediction
                        );

                        // Take maximum prediction across chunks
                        let current_max = final_predictions.get(mdl).unwrap_or(&0.0);
                        if prediction > *current_max {
                            final_predictions.insert(mdl.clone(), prediction);
                        }
                    } else {
                        log::debug!(
                            "🔍 Chunk {}: Model {}: insufficient features, skipping",
                            chunk_idx,
                            mdl
                        );
                    }
                } else {
                    log::debug!(
                        "🔍 Chunk {}: Model {}: unexpected prepared samples: {}",
                        chunk_idx,
                        mdl,
                        n_prepared_samples
                    );
                }
            }
        }

        // FIXME: This logic was zeroing out initial predictions
        // Zero predictions for first 5 frames during model initialization (matches Python)
        // for (model_name, prediction) in final_predictions.iter_mut() {
        //     if let Some(buffer) = self.prediction_buffer.get(model_name) {
        //         if buffer.len() < 5 {
        //             log::debug!(
        //                 "🔍 Zeroing prediction for {} (buffer length: {})",
        //                 model_name,
        //                 buffer.len()
        //             );
        //             *prediction = 0.0;
        //         }
        //     }
        // }

        // Update prediction buffers (matches Python)
        for (model_name, prediction) in &final_predictions {
            if let Some(buffer) = self.prediction_buffer.get_mut(model_name) {
                buffer.push_back(*prediction);
                if buffer.len() > 30 {
                    buffer.pop_front();
                }
            }
        }

        // Handle thresholds and debounce (simplified for now)
        if let Some(threshold_map) = threshold {
            for (model_name, prediction) in final_predictions.iter_mut() {
                if let Some(&threshold_value) = threshold_map.get(model_name) {
                    if *prediction < threshold_value {
                        *prediction = 0.0;
                    }
                }
            }
        }

        // Handle debounce time (simplified)
        if debounce_time > 0.0 {
            // TODO: Implement debounce logic similar to Python
        }

        Ok(final_predictions)
    }

    /// Get parent model name from label (for multi-class models)
    pub fn get_parent_model_from_label(&self, label: &str) -> Option<&str> {
        for (model_name, mapping) in self.class_mapping.iter() {
            if mapping.values().any(|v| v == label) {
                return Some(model_name);
            }
        }
        None
    }

    /// Get model input sizes (for debugging)
    pub fn get_model_inputs(&self) -> &HashMap<String, usize> {
        &self.model_inputs
    }

    /// Get preprocessor reference (for debugging)
    pub fn get_preprocessor(&self) -> &AudioFeatures {
        &self.preprocessor
    }

    /// Get mutable preprocessor reference (for debugging)
    pub fn get_preprocessor_mut(&mut self) -> &mut AudioFeatures {
        &mut self.preprocessor
    }

    /// Run model prediction on features (static to avoid borrowing issues)
    fn run_model_prediction_static(interpreter: &mut Interpreter, features: &[f32]) -> Result<f32> {
        // Debug: Print tensor info
        let input_tensor = interpreter.input(0).map_err(|e| {
            OpenWakeWordError::ProcessingError(format!("Failed to get input tensor: {}", e))
        })?;
        let expected_size = input_tensor.shape().dimensions().iter().product::<usize>();
        log::debug!(
            "🔍 Rust Stage 3 - Wakeword input: {} features, Expected by model: {}",
            features.len(),
            expected_size
        );

        if features.is_empty() {
            return Err(OpenWakeWordError::ProcessingError(
                "No features provided to model - feature extraction failed".to_string(),
            ));
        }

        // Debug: print wakeword input stats
        if !features.is_empty() {
            let mean = features.iter().sum::<f32>() / features.len() as f32;
            let min = features.iter().fold(f32::INFINITY, |a, &b| a.min(b));
            let max = features.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b));
            log::debug!(
                "🔍 Rust Stage 3 - Wakeword input stats: mean={:.6}, min={:.6}, max={:.6}",
                mean,
                min,
                max
            );
            log::debug!(
                "🔍 Rust Stage 3 - First 5 values: {:?}",
                &features[..features.len().min(5)]
            );
        }

        // Copy features to input tensor
        interpreter.copy(features, 0).map_err(|e| {
            OpenWakeWordError::ProcessingError(format!("Failed to set input: {}", e))
        })?;

        // Run inference
        interpreter
            .invoke()
            .map_err(|e| OpenWakeWordError::ProcessingError(format!("Inference failed: {}", e)))?;

        // Get output
        let output_tensor = interpreter.output(0).map_err(|e| {
            OpenWakeWordError::ProcessingError(format!("Failed to get output: {}", e))
        })?;

        let output_data = output_tensor.data::<f32>();
        if output_data.is_empty() {
            return Ok(0.0);
        }

        // Debug: print actual model output
        log::debug!(
            "🔍 Rust Stage 3 - Wakeword output: {} values: {:?}",
            output_data.len(),
            &output_data[..output_data.len().min(5)]
        );

        // Return first output value
        let result = output_data[0];
        log::debug!("🔍 Rust Stage 3 - Wakeword prediction: {}", result);
        Ok(result)
    }
}
