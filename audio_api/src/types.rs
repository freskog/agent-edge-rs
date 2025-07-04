use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WakewordConfig {
    pub model_path: String,
    pub threshold: f32,
    pub sensitivity: f32,
}
