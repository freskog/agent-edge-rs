use crate::audio_capture::AudioCaptureError;
use cpal::{DeviceNameError, DevicesError};
use std::io;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, EdgeError>;

#[derive(Error, Debug)]
pub enum EdgeError {
    #[error("Audio capture error: {0}")]
    Audio(String),

    #[error("Audio capture error: {0}")]
    AudioCapture(AudioCaptureError),

    #[error("Model loading error: {0}")]
    ModelLoadError(String),

    #[error("Processing error: {0}")]
    ProcessingError(String),

    #[error("Detection pipeline error: {0}")]
    Pipeline(String),

    #[error("Invalid input: {0}")]
    InvalidInput(String),

    #[error("VAD error: {0}")]
    VADError(String),

    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Unknown error: {0}")]
    Unknown(String),

    #[error("Audio device error: {0}")]
    AudioDevice(String),
}

impl From<DevicesError> for EdgeError {
    fn from(err: DevicesError) -> Self {
        EdgeError::AudioDevice(err.to_string())
    }
}

impl From<DeviceNameError> for EdgeError {
    fn from(err: DeviceNameError) -> Self {
        EdgeError::AudioDevice(err.to_string())
    }
}

// Add conversion from AudioCaptureError to EdgeError
impl From<crate::audio_capture::AudioCaptureError> for EdgeError {
    fn from(err: crate::audio_capture::AudioCaptureError) -> Self {
        EdgeError::AudioCapture(err)
    }
}

// Wrapper types for device errors
#[derive(Debug)]
pub struct DevicesErrorWrapper(pub DevicesError);

#[derive(Debug)]
pub struct DeviceNameErrorWrapper(pub DeviceNameError);

impl From<DevicesErrorWrapper> for EdgeError {
    fn from(err: DevicesErrorWrapper) -> Self {
        EdgeError::AudioDevice(err.0.to_string())
    }
}

impl From<DeviceNameErrorWrapper> for EdgeError {
    fn from(err: DeviceNameErrorWrapper) -> Self {
        EdgeError::AudioDevice(err.0.to_string())
    }
}
