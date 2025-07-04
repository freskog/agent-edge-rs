use tflitec::{
    interpreter::{Interpreter, Options},
    model::Model,
};

use crate::error::{EdgeError, Result};

pub struct EmbeddingModel {
    interpreter: Interpreter<'static>,
    expected_input_size: usize,
}

impl EmbeddingModel {
    pub fn new(model_path: &str) -> Result<Self> {
        log::info!("Loading embedding model from: {}", model_path);

        // Load the model and leak it for 'static lifetime
        let model = Box::leak(Box::new(Model::new(model_path).map_err(|e| {
            EdgeError::ModelLoadError(format!("Failed to load embedding model: {}", e))
        })?));

        // Create interpreter options - start with XNNPACK enabled (if compiled in)
        // Small models like embedding can actually be slower with multi-threading
        let mut options = Options::default();
        options.thread_count = 1;
        // XNNPACK is now enabled with pthreadpool linking

        // Create a temporary interpreter to inspect the model shape. If the
        // delegate crashes on this CPU fall back to a plain interpreter.
        let temp_interpreter = match Interpreter::new(model, Some(options.clone())) {
            Ok(i) => i,
            Err(e) => {
                log::warn!(
                    "Failed to create embedding interpreter with XNNPACK: {e}. Retrying without delegate."
                );

                let mut fallback = Options::default();
                fallback.thread_count = 1;
                // XNNPACK is now enabled with pthreadpool linking

                Interpreter::new(model, Some(fallback)).map_err(|e2| {
                    EdgeError::ModelLoadError(format!(
                        "Failed to create embedding interpreter without XNNPACK: {e2}"
                    ))
                })?
            }
        };

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

        let mut main_options = Options::default();
        main_options.thread_count = 1;
        // XNNPACK is now enabled with pthreadpool linking

        let interpreter = match Interpreter::new(model, Some(main_options)) {
            Ok(i) => i,
            Err(e) => {
                log::warn!(
                    "Failed to create main embedding interpreter with XNNPACK: {e}. Retrying without delegate."
                );

                let mut fallback = Options::default();
                fallback.thread_count = 1;
                #[cfg(feature = "xnnpack")]
                {
                    // XNNPACK is now enabled with pthreadpool linking
                }

                Interpreter::new(model, Some(fallback)).map_err(|e2| {
                    EdgeError::ModelLoadError(format!(
                        "Failed to create embedding interpreter without XNNPACK: {e2}"
                    ))
                })?
            }
        };

        interpreter.allocate_tensors().map_err(|e| {
            EdgeError::ModelLoadError(format!("Failed to allocate embedding tensors: {}", e))
        })?;

        log::info!("Embedding model loaded successfully");

        Ok(EmbeddingModel {
            interpreter,
            expected_input_size,
        })
    }

    pub fn predict(&mut self, features: &[f32]) -> Result<Vec<f32>> {
        // The embedding model expects accumulated melspectrogram frames (76 × 32 = 2432)
        if features.len() != self.expected_input_size {
            return Err(EdgeError::InvalidInput(format!(
                "Expected {} features for embedding model (76 frames × 32), got {}",
                self.expected_input_size,
                features.len()
            )));
        }

        // Set input and run inference
        self.interpreter.copy(features, 0).map_err(|e| {
            EdgeError::ProcessingError(format!("Failed to set embedding input: {}", e))
        })?;

        self.interpreter.invoke().map_err(|e| {
            EdgeError::ProcessingError(format!("Failed to run embedding inference: {}", e))
        })?;

        // Get output tensor and copy data
        let output_tensor = self.interpreter.output(0).map_err(|e| {
            EdgeError::ProcessingError(format!("Failed to get embedding output tensor: {}", e))
        })?;
        let output_data = output_tensor.data::<f32>().to_vec();

        Ok(output_data)
    }
}
