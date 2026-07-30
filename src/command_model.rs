//! ONNX command-wakeword classifiers (e.g. "hey mycroft stop").
//!
//! These are livekit-wakeword / openWakeWord-compatible classifier heads: they
//! consume the SAME `(n_frames, 96)` speech-embedding window that the tflite
//! wakeword models already compute (see `wakeword_utils::AudioFeatures`), and
//! emit a single activation score. We run them with `ort` (the ONNX Runtime
//! crate already linked via `voice_activity_detector`), so no extra native
//! dependency and no duplicate mel/embedding frontend.

use ort::session::{builder::GraphOptimizationLevel, Session};
use ort::value::Tensor;

use crate::wakeword_error::{OpenWakeWordError, Result};

/// A single ONNX command-wakeword classifier.
pub struct CommandClassifier {
    name: String,
    session: Session,
    n_frames: usize,
}

impl CommandClassifier {
    /// Load a classifier ONNX file. `n_frames` is the number of embedding
    /// timesteps the model expects (openWakeWord/livekit use 16 → input
    /// `[batch, 16, 96]`).
    pub fn load(name: &str, path: &str, n_frames: usize) -> Result<Self> {
        // Constrain ONNX Runtime to a single, non-spinning thread. These heads
        // are tiny ([1, n_frames, 96]) and run on the detection thread ~12.5x/sec;
        // ORT otherwise defaults its intra-op pool to every core and busy-waits
        // between calls, which pegged ~3 cores on the Pi Zero 2W. Running inline
        // on the caller with no spin keeps the cost to just the head inference.
        let session = Session::builder()
            .and_then(|b| b.with_optimization_level(GraphOptimizationLevel::Level3))
            .and_then(|b| b.with_intra_threads(1))
            .and_then(|b| b.with_inter_threads(1))
            .and_then(|b| b.with_intra_op_spinning(false))
            .and_then(|b| b.with_inter_op_spinning(false))
            .and_then(|b| b.commit_from_file(path))
            .map_err(|e| {
                OpenWakeWordError::ModelLoadError(format!(
                    "Failed to load command model {} from {}: {}",
                    name, path, e
                ))
            })?;
        Ok(Self {
            name: name.to_string(),
            session,
            n_frames,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn n_frames(&self) -> usize {
        self.n_frames
    }

    /// Score a flattened `(n_frames * 96)` embedding window. Returns the model's
    /// activation in `[0, 1]`.
    pub fn score(&mut self, features: &[f32]) -> Result<f32> {
        let expected = self.n_frames * 96;
        if features.len() < expected {
            return Ok(0.0); // not enough features accumulated yet
        }
        let shape = [1_i64, self.n_frames as i64, 96];
        let tensor = Tensor::from_array((shape, features[..expected].to_vec())).map_err(|e| {
            OpenWakeWordError::ProcessingError(format!("command tensor build failed: {}", e))
        })?;
        let outputs = self
            .session
            .run(ort::inputs![tensor])
            .map_err(|e| OpenWakeWordError::ProcessingError(format!("command inference failed: {}", e)))?;
        let arr = outputs[0]
            .try_extract_array::<f32>()
            .map_err(|e| OpenWakeWordError::ProcessingError(format!("command output extract failed: {}", e)))?;
        Ok(arr.iter().copied().next().unwrap_or(0.0))
    }
}
