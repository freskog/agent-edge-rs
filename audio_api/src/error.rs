use crate::audio_source::AudioCaptureError;
use cpal::{DeviceNameError, DevicesError};
use std::io;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, AudioError>;

#[derive(Error, Debug)]
pub enum AudioError {
    #[error("Audio capture error: {0}")]
    AudioCapture(AudioCaptureError),

    #[error("Audio sink error: {0}")]
    AudioSink(String),

    #[error("Audio device error: {0}")]
    AudioDevice(String),

    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("Unknown error: {0}")]
    Unknown(String),
}

impl From<DevicesError> for AudioError {
    fn from(err: DevicesError) -> Self {
        AudioError::AudioDevice(err.to_string())
    }
}

impl From<DeviceNameError> for AudioError {
    fn from(err: DeviceNameError) -> Self {
        AudioError::AudioDevice(err.to_string())
    }
}

// Add conversion from AudioCaptureError to AudioError
impl From<crate::audio_source::AudioCaptureError> for AudioError {
    fn from(err: crate::audio_source::AudioCaptureError) -> Self {
        AudioError::AudioCapture(err)
    }
}

// Add conversion from audio_sink::AudioError to AudioError
impl From<crate::audio_sink::AudioError> for AudioError {
    fn from(err: crate::audio_sink::AudioError) -> Self {
        AudioError::AudioSink(err.to_string())
    }
}
