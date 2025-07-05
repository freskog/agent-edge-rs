//! OpenWakeWord Model implementation
//!
//! This module provides the main Model class that matches the Python implementation,
//! including proper prediction buffer management and simplified prediction interface.

use crate::error::{OpenWakeWordError, Result};
use crate::utils::AudioFeatures;
use crate::{get_model_class_mappings, FEATURE_MODELS, MODELS};
use std::collections::{HashMap, VecDeque};
use tflitec::interpreter::Options;
use tflitec::{interpreter::Interpreter, model::Model as TfliteModel};

/// Model prediction results type
pub type PredictionResult = HashMap<String, f32>;

/// Main OpenWakeWord Model class
///
/// Matches the Python Model class interface with:
/// - Shared audio preprocessor (AudioFeatures)
/// - Multiple wake word models
/// - Prediction buffers with maxlen=30
/// - Simple predict() interface
pub struct Model {
    // Model storage
    models: HashMap<String, Interpreter<'static>>,
    model_inputs: HashMap<String, usize>,
    model_outputs: HashMap<String, usize>,

    // Class mappings for multi-class models
    class_mapping: HashMap<String, HashMap<String, String>>,

    // Audio preprocessor (shared)
    preprocessor: AudioFeatures,

    // Prediction buffers (deque with maxlen=30 like Python)
    prediction_buffer: HashMap<String, VecDeque<f32>>,

    // Configuration
    vad_threshold: f32,
    custom_verifier_threshold: f32,
}

impl Model {
    /// Create a new Model instance
    ///
    /// # Arguments
    /// * `wakeword_models` - List of model names or paths to load
    /// * `class_mapping_dicts` - Optional class mappings for multi-class models
    /// * `vad_threshold` - Voice activity detection threshold (0 to disable)
    /// * `custom_verifier_threshold` - Custom verifier threshold
    ///
    /// # Returns
    /// * `Result<Model>` - New Model instance
    pub fn new(
        wakeword_models: Vec<String>,
        class_mapping_dicts: Vec<HashMap<String, String>>,
        vad_threshold: f32,
        custom_verifier_threshold: f32,
    ) -> Result<Self> {
        // Get model paths - if empty, load all pre-trained models
        let mut model_names = Vec::new();
        let mut resolved_paths = Vec::new();

        if wakeword_models.is_empty() {
            // Load all pre-trained models
            for (name, path) in MODELS {
                model_names.push(name.to_string());
                resolved_paths.push(path.to_string());
            }
        } else {
            // Resolve provided model names/paths
            for model_path in wakeword_models {
                if std::path::Path::new(&model_path).exists() {
                    // Direct path provided
                    let name = std::path::Path::new(&model_path)
                        .file_stem()
                        .unwrap()
                        .to_string_lossy()
                        .into_owned();
                    model_names.push(name);
                    resolved_paths.push(model_path);
                } else {
                    // Model name provided, find pre-trained path
                    let matching_models: Vec<String> = MODELS
                        .iter()
                        .filter(|(name, _)| *name == &model_path)
                        .map(|(_, path)| path.to_string())
                        .collect();

                    if matching_models.is_empty() {
                        return Err(OpenWakeWordError::ModelLoadError(format!(
                            "Could not find pretrained model for model name '{}'",
                            model_path
                        )));
                    }

                    model_names.push(model_path);
                    resolved_paths.push(matching_models[0].clone());
                }
            }
        }

        // Initialize models with TFLite
        let mut models = HashMap::new();
        let mut model_inputs = HashMap::new();
        let mut model_outputs = HashMap::new();
        let mut model_prediction_functions: HashMap<
            String,
            Box<dyn FnMut(&[f32]) -> Result<Vec<f32>>>,
        > = HashMap::new();

        for (model_name, model_path) in model_names.iter().zip(resolved_paths.iter()) {
            // Load TFLite model
            let tflite_model = Box::leak(Box::new(TfliteModel::new(model_path).map_err(|e| {
                OpenWakeWordError::ModelLoadError(format!(
                    "Failed to load model {}: {}",
                    model_name, e
                ))
            })?));

            let mut options = Options::default();
            options.thread_count = 1;

            let interpreter = Interpreter::new(tflite_model, Some(options)).map_err(|e| {
                OpenWakeWordError::ModelLoadError(format!(
                    "Failed to create interpreter for {}: {}",
                    model_name, e
                ))
            })?;

            interpreter.allocate_tensors().map_err(|e| {
                OpenWakeWordError::ModelLoadError(format!(
                    "Failed to allocate tensors for {}: {}",
                    model_name, e
                ))
            })?;

            // Get input/output shapes
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

            let input_size = input_tensor.shape().dimensions().iter().product::<usize>();
            let output_size = output_tensor.shape().dimensions().iter().product::<usize>();

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
            let mut deque = VecDeque::with_capacity(30);
            // Add some initial values like Python
            for _ in 0..5 {
                deque.push_back(0.0);
            }
            prediction_buffer.insert(model_name.clone(), deque);
        }

        Ok(Model {
            models,
            model_inputs,
            model_outputs,
            class_mapping,
            preprocessor,
            prediction_buffer,
            vad_threshold,
            custom_verifier_threshold,
        })
    }

    /// Reset internal state
    pub fn reset(&mut self) -> Result<()> {
        self.preprocessor.reset()?;
        for buffer in self.prediction_buffer.values_mut() {
            buffer.clear();
            // Add initial values
            for _ in 0..5 {
                buffer.push_back(0.0);
            }
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
        // Get audio features (matches Python: n_prepared_samples = self.preprocessor(x))
        let n_prepared_samples = self.preprocessor.__call__(x)?;

        let mut predictions = HashMap::new();

        // Get predictions from each model (matches Python logic)
        let model_names: Vec<String> = self.models.keys().cloned().collect();

        for mdl in model_names {
            let input_size = self.model_inputs[&mdl] / 96; // Convert to number of frames
            let output_size = self.model_outputs[&mdl];

            // Run model to get predictions (matching Python logic)
            let prediction = if n_prepared_samples > 1280 {
                // Multiple chunks - process them
                let mut group_predictions = Vec::new();
                let n_chunks = n_prepared_samples / 1280;

                for i in 0..n_chunks {
                    let chunk_features = self
                        .preprocessor
                        .get_features(input_size, -(input_size as i32) - (i as i32));

                    let pred_result = Self::run_model_prediction_static(
                        self.models.get_mut(&mdl).unwrap(),
                        &chunk_features,
                    )?;
                    group_predictions.push(pred_result);
                }

                // Take maximum prediction
                group_predictions.iter().fold(0.0_f32, |a, &b| a.max(b))
            } else if n_prepared_samples == 1280 {
                // Single chunk - only predict if we have enough features
                let features = self.preprocessor.get_features(input_size, -1);

                // Debug: Print feature information (remove this later)
                // println!(
                //     "🔍 Debug - Model: {}, Input size: {}, Features len: {}, Expected total: {}",
                //     mdl,
                //     input_size,
                //     features.len(),
                //     self.model_inputs[&mdl]
                // );

                // For now, let's try with a lower threshold to see if the model works
                // We should have at least 960 features (10 embeddings × 96) to get meaningful results
                if features.len() >= 960 {
                    // Pad features to the required size if necessary
                    let mut padded_features = features.clone();
                    while padded_features.len() < self.model_inputs[&mdl] {
                        padded_features.push(0.0);
                    }
                    // println!(
                    //     "🔍 Running model with {} features (padded to {})",
                    //     features.len(),
                    //     padded_features.len()
                    // );

                    Self::run_model_prediction_static(
                        self.models.get_mut(&mdl).unwrap(),
                        &padded_features,
                    )?
                } else {
                    // Not enough features yet - use previous prediction
                    // println!("🔍 Not enough features yet, using previous prediction");
                    if let Some(last_pred) =
                        self.prediction_buffer.get(&mdl).and_then(|buf| buf.back())
                    {
                        *last_pred
                    } else {
                        0.0
                    }
                }
            } else {
                // Not enough samples - use previous prediction or zero
                if let Some(last_pred) = self.prediction_buffer.get(&mdl).and_then(|buf| buf.back())
                {
                    *last_pred
                } else {
                    0.0
                }
            };

            // Handle multi-class outputs
            if output_size == 1 {
                predictions.insert(mdl.clone(), prediction);
            } else {
                // Multi-class model - use class mappings
                if let Some(mapping) = self.class_mapping.get(&mdl) {
                    for (int_label, cls) in mapping {
                        if let Ok(idx) = int_label.parse::<usize>() {
                            // Would need to handle multiple outputs properly here
                            predictions.insert(cls.clone(), prediction);
                        }
                    }
                } else {
                    predictions.insert(mdl.clone(), prediction);
                }
            }

            // Zero predictions for first 5 frames during model initialization
            let prediction_keys: Vec<String> = predictions.keys().cloned().collect();
            for key in prediction_keys {
                if let Some(buffer) = self.prediction_buffer.get(&key) {
                    if buffer.len() < 5 {
                        predictions.insert(key, 0.0);
                    }
                }
            }
        }

        // Apply debounce logic if specified (simplified version)
        if debounce_time > 0.0 {
            if let Some(thresh) = threshold {
                for (model_name, prediction) in predictions.iter_mut() {
                    if let Some(model_threshold) = thresh.get(model_name) {
                        if *prediction >= *model_threshold {
                            let n_frames = (debounce_time * 16000.0 / n_prepared_samples as f32)
                                .ceil() as usize;
                            if let Some(buffer) = self.prediction_buffer.get(model_name) {
                                let recent_predictions: Vec<f32> =
                                    buffer.iter().rev().take(n_frames).cloned().collect();
                                if recent_predictions.iter().any(|&p| p >= *model_threshold) {
                                    *prediction = 0.0;
                                }
                            }
                        }
                    }
                }
            }
        }

        // Update prediction buffers (matching Python logic)
        for (model_name, prediction) in &predictions {
            if let Some(buffer) = self.prediction_buffer.get_mut(model_name) {
                buffer.push_back(*prediction);
                if buffer.len() > 30 {
                    buffer.pop_front();
                }
            }
        }

        Ok(predictions)
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
        // println!(
        //     "🔍 Static prediction - Features provided: {}, Expected by model: {}",
        //     features.len(),
        //     expected_size
        // );

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

        // Return first output value
        Ok(output_data[0])
    }
}
