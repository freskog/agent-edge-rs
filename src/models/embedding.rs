use std::sync::Mutex;
use tflitec::{
    interpreter::{Interpreter, Options},
    model::Model,
};

use crate::error::{EdgeError, Result};

pub struct EmbeddingModel {
    interpreter: Mutex<Interpreter<'static>>,
    expected_input_size: usize,
}

impl EmbeddingModel {
    pub fn new(model_path: &str) -> Result<Self> {
        log::info!("Loading embedding model from: {}", model_path);

        // Load the model and leak it for 'static lifetime
        let model = Box::leak(Box::new(Model::new(model_path).map_err(|e| {
            EdgeError::ModelLoadError(format!("Failed to load embedding model: {}", e))
        })?));

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

        // Drop the temporary interpreter
        drop(temp_interpreter);

        // Create the main interpreter
        let interpreter = Interpreter::new(model, Some(options)).map_err(|e| {
            EdgeError::ModelLoadError(format!("Failed to create embedding interpreter: {}", e))
        })?;

        interpreter.allocate_tensors().map_err(|e| {
            EdgeError::ModelLoadError(format!("Failed to allocate embedding tensors: {}", e))
        })?;

        log::info!("Embedding model loaded successfully");

        Ok(EmbeddingModel {
            interpreter: Mutex::new(interpreter),
            expected_input_size,
        })
    }

    pub fn predict(&self, features: &[f32]) -> Result<Vec<f32>> {
        // The embedding model expects accumulated melspectrogram frames (76 × 32 = 2432)
        if features.len() != self.expected_input_size {
            return Err(EdgeError::InvalidInput(format!(
                "Expected {} features for embedding model (76 frames × 32), got {}",
                self.expected_input_size,
                features.len()
            )));
        }

        log::debug!("Embedding model processing {} features", features.len());

        // Get the interpreter
        let interpreter = self.interpreter.lock().map_err(|e| {
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
