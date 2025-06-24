//! Machine Learning Models for Wakeword Detection
//!
//! This module provides production-ready ML models for edge AI wakeword detection
//! using TensorFlow Lite with the tflitec crate.

// Core production modules
pub mod embedding;
pub mod melspectrogram;
pub mod wakeword;

// Re-export main types for convenient access
pub use melspectrogram::{MelSpectrogramConfig, MelSpectrogramModel};
pub use wakeword::WakewordDetection;
