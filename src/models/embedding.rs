use tflitec::{
    interpreter::{Interpreter, Options},
    model::Model,
};

use crate::error::{EdgeError, Result};

pub struct EmbeddingModel<'a> {
    model: Model<'a>,
    expected_input_size: usize,
    expected_output_size: usize,
}

impl<'a> EmbeddingModel<'a> {
    pub fn new(model_path: &str) -> Result<Self> {
        log::info!("Loading embedding model from: {}", model_path);

        let model = Model::new(model_path).map_err(|e| {
            EdgeError::ModelLoadError(format!("Failed to load embedding model: {}", e))
        })?;

        // Create interpreter to inspect model shape
        let mut options = Options::default();
        options.thread_count = 1;

        let interpreter = Interpreter::new(&model, Some(options)).map_err(|e| {
            EdgeError::ModelLoadError(format!("Failed to create embedding interpreter: {}", e))
        })?;

        // Allocate tensors first so we can inspect the shapes
        interpreter.allocate_tensors().map_err(|e| {
            EdgeError::ModelLoadError(format!("Failed to allocate embedding tensors: {}", e))
        })?;

        // Get input and output shapes with proper lifetime management
        let input_tensor = interpreter.input(0).map_err(|e| {
            EdgeError::ModelLoadError(format!("Failed to get embedding input tensor: {}", e))
        })?;
        let output_tensor = interpreter.output(0).map_err(|e| {
            EdgeError::ModelLoadError(format!("Failed to get embedding output tensor: {}", e))
        })?;

        let input_shape = input_tensor.shape();
        let output_shape = output_tensor.shape();

        let expected_input_size = input_shape.dimensions().iter().product::<usize>();
        let expected_output_size = output_shape.dimensions().iter().product::<usize>();

        log::info!(
            "Embedding model input shape: {:?} (size: {})",
            input_shape.dimensions(),
            expected_input_size
        );
        log::info!(
            "Embedding model output shape: {:?} (size: {})",
            output_shape.dimensions(),
            expected_output_size
        );

        // Drop the interpreter before moving model
        drop(interpreter);

        Ok(EmbeddingModel {
            model,
            expected_input_size,
            expected_output_size,
        })
    }

    pub fn get_expected_input_size(&self) -> usize {
        self.expected_input_size
    }

    pub fn get_expected_output_size(&self) -> usize {
        self.expected_output_size
    }

    pub fn predict(&self, features: &[f32]) -> Result<Vec<f32>> {
        if features.len() != self.expected_input_size {
            return Err(EdgeError::InvalidInput(format!(
                "Expected {} features, got {}",
                self.expected_input_size,
                features.len()
            )));
        }

        // Create interpreter options
        let mut options = Options::default();
        options.thread_count = 1;

        // Create interpreter
        let interpreter = Interpreter::new(&self.model, Some(options)).map_err(|e| {
            EdgeError::ProcessingError(format!("Failed to create embedding interpreter: {}", e))
        })?;

        // Reshape input tensor to [1, 76, 32, 1] as expected by the model
        let input_shape = tflitec::tensor::Shape::new(vec![1, 76, 32, 1]);
        interpreter.resize_input(0, input_shape).map_err(|e| {
            EdgeError::ProcessingError(format!("Failed to resize embedding input tensor: {}", e))
        })?;

        // Allocate tensors after resizing
        interpreter.allocate_tensors().map_err(|e| {
            EdgeError::ProcessingError(format!("Failed to allocate embedding tensors: {}", e))
        })?;

        // Set input tensor data
        interpreter.copy(features, 0).map_err(|e| {
            EdgeError::ProcessingError(format!("Failed to set embedding input: {}", e))
        })?;

        // Run inference
        interpreter.invoke().map_err(|e| {
            EdgeError::ProcessingError(format!("Failed to run embedding inference: {}", e))
        })?;

        // Get output tensor
        let output_tensor = interpreter.output(0).unwrap();
        let output_data = output_tensor.data::<f32>().to_vec();

        Ok(output_data)
    }
}
