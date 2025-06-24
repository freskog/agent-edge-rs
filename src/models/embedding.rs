use std::sync::{Mutex, OnceLock};
use tflitec::{
    interpreter::{Interpreter, Options},
    model::Model,
};

use crate::error::{EdgeError, Result};

// Static storage for the model and interpreter
static EMBEDDING_MODEL: OnceLock<Model<'static>> = OnceLock::new();
static EMBEDDING_INTERPRETER: OnceLock<Mutex<Interpreter<'static>>> = OnceLock::new();
static EMBEDDING_INPUT_SIZE: OnceLock<usize> = OnceLock::new();

pub struct EmbeddingModel;

impl EmbeddingModel {
    pub fn new(model_path: &str) -> Result<Self> {
        log::info!("Loading embedding model from: {}", model_path);

        // Initialize the static model
        let model = Model::new(model_path).map_err(|e| {
            EdgeError::ModelLoadError(format!("Failed to load embedding model: {}", e))
        })?;

        let model = EMBEDDING_MODEL.get_or_init(|| model);

        // Create interpreter options
        let mut options = Options::default();
        options.thread_count = 1;

        // Create a temporary interpreter to inspect the model shape
        let temp_interpreter = Interpreter::new(model, Some(options.clone())).map_err(|e| {
            EdgeError::ModelLoadError(format!("Failed to create embedding interpreter: {}", e))
        })?;

        // Allocate tensors to inspect the shapes
        temp_interpreter.allocate_tensors().map_err(|e| {
            EdgeError::ModelLoadError(format!("Failed to allocate embedding tensors: {}", e))
        })?;

        // Get input shape and calculate expected input size
        let input_tensor = temp_interpreter.input(0).map_err(|e| {
            EdgeError::ModelLoadError(format!("Failed to get embedding input tensor: {}", e))
        })?;

        let input_shape = input_tensor.shape();
        let expected_input_size = input_shape.dimensions().iter().product::<usize>();

        log::info!(
            "Embedding model input shape: {:?} (size: {})",
            input_shape.dimensions(),
            expected_input_size
        );

        // Store the expected input size
        EMBEDDING_INPUT_SIZE
            .set(expected_input_size)
            .map_err(|_| EdgeError::ModelLoadError("Failed to set input size".to_string()))?;

        // Drop the temporary interpreter
        drop(temp_interpreter);

        // Create the static interpreter
        let interpreter = Interpreter::new(model, Some(options)).map_err(|e| {
            EdgeError::ModelLoadError(format!("Failed to create embedding interpreter: {}", e))
        })?;

        interpreter.allocate_tensors().map_err(|e| {
            EdgeError::ModelLoadError(format!("Failed to allocate embedding tensors: {}", e))
        })?;

        // Store the interpreter in a mutex for thread safety
        EMBEDDING_INTERPRETER
            .set(Mutex::new(interpreter))
            .map_err(|_| {
                EdgeError::ModelLoadError("Failed to initialize interpreter".to_string())
            })?;

        log::info!("Embedding model loaded successfully");

        Ok(EmbeddingModel)
    }

    pub fn predict(&self, features: &[f32]) -> Result<Vec<f32>> {
        // Get the expected input size from the model (should be 2432 for 76 frames × 32 features)
        let expected_input_size = EMBEDDING_INPUT_SIZE.get().ok_or_else(|| {
            EdgeError::ProcessingError("Embedding model not initialized".to_string())
        })?;

        // The embedding model expects accumulated melspectrogram frames (76 × 32 = 2432)
        if features.len() != *expected_input_size {
            return Err(EdgeError::InvalidInput(format!(
                "Expected {} features for embedding model (76 frames × 32), got {}",
                *expected_input_size,
                features.len()
            )));
        }

        log::debug!("Embedding model processing {} features", features.len());

        // Get the static interpreter
        let interpreter_mutex = EMBEDDING_INTERPRETER.get().ok_or_else(|| {
            EdgeError::ProcessingError("Embedding model not initialized".to_string())
        })?;

        let interpreter = interpreter_mutex.lock().map_err(|e| {
            EdgeError::ProcessingError(format!("Failed to lock interpreter: {}", e))
        })?;

        // Set input and run inference
        interpreter.copy(features, 0).map_err(|e| {
            EdgeError::ProcessingError(format!("Failed to set embedding input: {}", e))
        })?;

        interpreter.invoke().map_err(|e| {
            EdgeError::ProcessingError(format!("Failed to run embedding inference: {}", e))
        })?;

        // Get output
        let output_tensor = interpreter.output(0).unwrap();
        let output_data = output_tensor.data::<f32>().to_vec();

        log::debug!("Embedding model produced {} features", output_data.len());

        Ok(output_data)
    }
}
