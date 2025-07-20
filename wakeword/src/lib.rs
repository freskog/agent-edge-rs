// Copyright 2024 - OpenWakeWord Rust Port
// Licensed under the Apache License, Version 2.0

//! # OpenWakeWord Rust Port
//!
//! A direct port of the Python OpenWakeWord implementation to Rust, providing
//! wake word detection using TensorFlow Lite models.
//!
//! This implementation closely mirrors the Python version's structure and API
//! for better compatibility and performance.

pub mod error;
// TODO: Convert to TCP protocol
// pub mod grpc_client;
pub mod model;
pub mod utils;
pub mod xnnpack_fix;

pub use error::*;
pub use model::*;
pub use utils::*;

#[cfg(test)]
mod tests;

pub mod test_utils;

use std::collections::HashMap;

/// Feature models configuration (melspectrogram and embedding)
pub const FEATURE_MODELS: &[(&str, &str)] = &[
    ("embedding", "models/embedding_model.tflite"),
    ("melspectrogram", "models/melspectrogram.tflite"),
];

/// Available wake word models
pub const MODELS: &[(&str, &str)] = &[
    ("alexa", "models/alexa_v0.1.tflite"),
    ("hey_mycroft", "models/hey_mycroft_v0.1.tflite"),
    ("hey_jarvis", "models/hey_jarvis_v0.1.tflite"),
    ("hey_rhasspy", "models/hey_rhasspy_v0.1.tflite"),
    ("timer", "models/timer_v0.1.tflite"),
    ("weather", "models/weather_v0.1.tflite"),
];

/// Get pre-trained model paths for TFLite models
pub fn get_pretrained_model_paths() -> Vec<String> {
    MODELS.iter().map(|(_, path)| path.to_string()).collect()
}

/// Default model class mappings for multi-class models
pub fn get_model_class_mappings() -> HashMap<String, HashMap<String, String>> {
    let mut mappings = HashMap::new();

    // Timer model has multiple classes
    let mut timer_mapping = HashMap::new();
    timer_mapping.insert("1".to_string(), "1_minute_timer".to_string());
    timer_mapping.insert("2".to_string(), "5_minute_timer".to_string());
    timer_mapping.insert("3".to_string(), "10_minute_timer".to_string());
    timer_mapping.insert("4".to_string(), "20_minute_timer".to_string());
    timer_mapping.insert("5".to_string(), "30_minute_timer".to_string());
    timer_mapping.insert("6".to_string(), "1_hour_timer".to_string());
    mappings.insert("timer".to_string(), timer_mapping);

    mappings
}
