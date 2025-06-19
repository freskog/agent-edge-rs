//! Machine Learning Models for Wakeword Detection
//!
//! This module provides production-ready ML models for edge AI wakeword detection
//! using TensorFlow Lite with the tflitec crate.

// Core production modules
pub mod melspectrogram;
pub mod wakeword;

// Re-export main types for convenient access
pub use melspectrogram::{MelSpectrogramConfig, MelSpectrogramProcessor};
pub use wakeword::{
    initialize_detector, process_audio_global, WakewordConfig, WakewordDetection, WakewordDetector,
};
