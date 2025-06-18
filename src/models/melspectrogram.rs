use crate::audio::AudioBuffer;
use crate::error::{EdgeError, Result};
use tflite::ops::builtin::BuiltinOpResolver;
use tflite::{FlatBufferModel, InterpreterBuilder};

/// Embedded melspectrogram model (baked into binary for edge deployment)
const MELSPECTROGRAM_MODEL_BYTES: &[u8] = include_bytes!("../../models/melspectrogram.tflite");

/// Configuration for melspectrogram processing
#[derive(Debug, Clone)]
pub struct MelSpectrogramConfig {
    /// Sample rate for audio processing (should match audio capture)
    pub sample_rate: u32,
    /// Duration of each chunk in milliseconds
    pub chunk_duration_ms: u32,
    /// Number of mel frequency bins
    pub n_mels: usize,
}

impl Default for MelSpectrogramConfig {
    fn default() -> Self {
        Self {
            sample_rate: 16000,    // Standard for speech processing
            chunk_duration_ms: 80, // 80ms chunks as specified
            n_mels: 80,            // Typical mel filterbank size
        }
    }
}

/// Melspectrogram processor that converts 80ms audio chunks to mel spectrograms
pub struct MelSpectrogramProcessor {
    config: MelSpectrogramConfig,
    interpreter: tflite::Interpreter,
    chunk_size_samples: usize,
    input_index: i32,
    output_index: i32,
}

impl MelSpectrogramProcessor {
    /// Create a new melspectrogram processor with embedded model
    pub fn new(config: MelSpectrogramConfig) -> Result<Self> {
        let chunk_size_samples = (config.sample_rate * config.chunk_duration_ms / 1000) as usize;

        log::info!(
            "Loading embedded melspectrogram model ({} bytes)",
            MELSPECTROGRAM_MODEL_BYTES.len()
        );

        // Load the embedded model
        let model = FlatBufferModel::build_from_buffer(MELSPECTROGRAM_MODEL_BYTES.to_vec())
            .map_err(|e| EdgeError::Model(format!("Failed to load melspectrogram model: {}", e)))?;

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

        // Get tensor info for logging
        let input_tensor = interpreter
            .tensor_info(input_index)
            .ok_or_else(|| EdgeError::Model("Failed to get input tensor info".to_string()))?;
        let output_tensor = interpreter
            .tensor_info(output_index)
            .ok_or_else(|| EdgeError::Model("Failed to get output tensor info".to_string()))?;

        log::info!("MelSpectrogram processor initialized:");
        log::info!(
            "  - Chunk duration: {}ms ({} samples at {}Hz)",
            config.chunk_duration_ms,
            chunk_size_samples,
            config.sample_rate
        );
        log::info!("  - Mel bins: {}", config.n_mels);
        log::info!("  - Input tensor shape: {:?}", input_tensor.dims);
        log::info!("  - Output tensor shape: {:?}", output_tensor.dims);

        Ok(Self {
            config,
            interpreter,
            chunk_size_samples,
            input_index,
            output_index,
        })
    }

    /// Process an 80ms audio chunk and return mel spectrogram features
    pub fn process_chunk(&mut self, audio_chunk: &AudioBuffer) -> Result<Vec<f32>> {
        // Validate input size
        if audio_chunk.len() != self.chunk_size_samples {
            return Err(EdgeError::Model(format!(
                "Invalid chunk size: expected {} samples, got {}",
                self.chunk_size_samples,
                audio_chunk.len()
            )));
        }

        // Get input tensor data and copy audio data
        let input_data: &mut [f32] = self
            .interpreter
            .tensor_data_mut(self.input_index)
            .map_err(|e| EdgeError::Model(format!("Failed to get input tensor data: {}", e)))?;

        if input_data.len() < audio_chunk.len() {
            return Err(EdgeError::Model(
                "Input tensor too small for audio chunk".to_string(),
            ));
        }

        input_data[..audio_chunk.len()].copy_from_slice(audio_chunk);

        // Run inference
        self.interpreter
            .invoke()
            .map_err(|e| EdgeError::Model(format!("Inference failed: {}", e)))?;

        // Get output tensor data
        let output_data: &[f32] = self
            .interpreter
            .tensor_data(self.output_index)
            .map_err(|e| EdgeError::Model(format!("Failed to get output tensor data: {}", e)))?;

        // Return mel spectrogram features
        Ok(output_data.to_vec())
    }

    /// Get the expected chunk size in samples
    pub fn chunk_size_samples(&self) -> usize {
        self.chunk_size_samples
    }

    /// Get the expected chunk duration in milliseconds
    pub fn chunk_duration_ms(&self) -> u32 {
        self.config.chunk_duration_ms
    }

    /// Get the number of mel frequency bins in the output
    pub fn n_mels(&self) -> usize {
        self.config.n_mels
    }

    /// Get output feature dimensions (for accumulating frames for wakeword model)
    pub fn output_shape(&self) -> Result<Vec<usize>> {
        let output_tensor = self
            .interpreter
            .tensor_info(self.output_index)
            .ok_or_else(|| EdgeError::Model("Failed to get output tensor info".to_string()))?;
        Ok(output_tensor.dims.iter().map(|&d| d as usize).collect())
    }
}
