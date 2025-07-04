use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    Device, FromSample, Host, Sample, SampleFormat, SizedSample, Stream as CpalStream,
};
use thiserror::Error;
use tokio::sync::mpsc as tokio_mpsc;

pub const CHUNK_SIZE: usize = 1280; // Fixed chunk size that works with Silero

#[derive(Error, Debug)]
pub enum AudioCaptureError {
    #[error("No audio devices found")]
    NoDevices,
    #[error("Audio device error: {0}")]
    Device(String),
    #[error("Audio stream error: {0}")]
    Stream(String),
    #[error("Configuration error: {0}")]
    Config(String),
}

/// Audio capture configuration
#[derive(Debug, Clone)]
pub struct AudioCaptureConfig {
    /// Device ID to capture from (None = default device)
    pub device_id: Option<String>,
    /// Channel to capture (0-based index)
    pub channel: u32,
    pub sample_rate: u32,
    pub channels: u16,
}

impl Default for AudioCaptureConfig {
    fn default() -> Self {
        Self {
            device_id: None,
            channel: 0,
            sample_rate: 16000, // 16kHz for audio processing
            channels: 1,        // Mono
        }
    }
}

/// Audio device information
#[derive(Debug, Clone)]
pub struct AudioDeviceInfo {
    pub name: String,
    pub id: String,
    pub is_default: bool,
    pub channel_count: u32,
}

/// Simple audio capture that streams chunks directly
pub struct AudioCapture {
    _stream: CpalStream, // Keep stream alive
    _host: Host,         // Keep host alive
}

impl AudioCapture {
    /// Create a new audio capture that sends chunks via a channel
    pub fn new(
        config: AudioCaptureConfig,
        sender: tokio_mpsc::Sender<[f32; CHUNK_SIZE]>,
    ) -> Result<Self, AudioCaptureError> {
        let host = cpal::default_host();
        log::info!("🎤 Initializing audio capture with host: {:?}", host.id());

        // Get the device
        let device = if let Some(id) = &config.device_id {
            host.devices()
                .map_err(|e| AudioCaptureError::Device(e.to_string()))?
                .find(|d| d.name().map(|n| n == *id).unwrap_or(false))
                .ok_or_else(|| AudioCaptureError::Device(format!("Device not found: {}", id)))?
        } else {
            host.default_input_device()
                .ok_or_else(|| AudioCaptureError::Device("No default input device found".into()))?
        };

        log::info!("🎤 Using input device: {:?}", device.name());

        let supported_config = device
            .default_input_config()
            .map_err(|e| AudioCaptureError::Config(e.to_string()))?;

        // Verify channel selection is valid
        if config.channel >= u32::from(supported_config.channels()) {
            return Err(AudioCaptureError::Config(format!(
                "Selected channel {} is not available (device has {} channels)",
                config.channel,
                supported_config.channels()
            )));
        }

        let stream_config = supported_config.config();
        let err_fn = move |err| {
            log::error!("Audio stream error: {}", err);
        };

        // Log the format being used
        log::info!(
            "🎤 Audio capture configured: {} channels @ {}Hz (format: {:?})",
            stream_config.channels,
            stream_config.sample_rate.0,
            supported_config.sample_format()
        );

        let channel = config.channel;
        let channels = stream_config.channels as usize;

        // Build the stream with the appropriate sample format
        let stream = match supported_config.sample_format() {
            SampleFormat::I16 => Self::create_input_stream::<i16>(
                &device,
                &stream_config,
                channel,
                channels,
                sender,
                err_fn,
            )?,
            SampleFormat::U16 => Self::create_input_stream::<u16>(
                &device,
                &stream_config,
                channel,
                channels,
                sender,
                err_fn,
            )?,
            SampleFormat::F32 => Self::create_input_stream::<f32>(
                &device,
                &stream_config,
                channel,
                channels,
                sender,
                err_fn,
            )?,
            _ => {
                return Err(AudioCaptureError::Config(
                    "Unsupported sample format".into(),
                ))
            }
        };

        stream
            .play()
            .map_err(|e| AudioCaptureError::Stream(e.to_string()))?;

        Ok(Self {
            _stream: stream,
            _host: host,
        })
    }

    fn create_input_stream<T>(
        device: &Device,
        config: &cpal::StreamConfig,
        channel: u32,
        channels: usize,
        sender: tokio_mpsc::Sender<[f32; CHUNK_SIZE]>,
        err_fn: impl FnMut(cpal::StreamError) + Send + 'static + Copy,
    ) -> Result<CpalStream, AudioCaptureError>
    where
        T: Sample + SizedSample + Send + Sync + 'static,
        f32: FromSample<T>,
    {
        let mut sample_buffer = [0.0f32; CHUNK_SIZE];
        let mut sample_count = 0;

        device
            .build_input_stream(
                config,
                move |data: &[T], _: &cpal::InputCallbackInfo| {
                    // Extract the specified channel and convert to f32
                    for frame in data.chunks(channels) {
                        if let Some(sample) = frame.get(channel as usize) {
                            let value = f32::from_sample(*sample);

                            // Add sample to buffer
                            if sample_count < CHUNK_SIZE {
                                sample_buffer[sample_count] = value;
                                sample_count += 1;
                            }

                            // If we have enough samples, send a chunk
                            if sample_count >= CHUNK_SIZE {
                                let chunk = sample_buffer;
                                sample_buffer = [0.0f32; CHUNK_SIZE];
                                sample_count = 0;

                                // Send chunk (ignore errors - receiver might be gone)
                                let _ = sender.send(chunk);
                            }
                        }
                    }
                },
                err_fn,
                None,
            )
            .map_err(|e| AudioCaptureError::Stream(e.to_string()))
    }

    pub fn list_devices() -> Result<Vec<AudioDeviceInfo>, AudioCaptureError> {
        let host = cpal::default_host();
        let devices = host
            .devices()
            .map_err(|e| AudioCaptureError::Device(e.to_string()))?;

        let default_device = host.default_input_device();

        let mut result = Vec::new();
        for device in devices {
            if let Ok(name) = device.name() {
                let config = device
                    .default_input_config()
                    .map_err(|e| AudioCaptureError::Config(e.to_string()))?;

                result.push(AudioDeviceInfo {
                    name: name.clone(),
                    id: name.clone(),
                    is_default: default_device
                        .as_ref()
                        .map(|d| d.name().unwrap_or_default())
                        == Some(name),
                    channel_count: u32::from(config.channels()),
                });
            }
        }

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn test_audio_capture_creation() {
        let (tx, _rx) = mpsc::channel(100);

        match AudioCapture::new(AudioCaptureConfig::default(), tx) {
            Ok(_capture) => {
                // Successfully created
            }
            Err(e) => {
                println!(
                    "Audio device not available in test environment - this is expected: {}",
                    e
                );
            }
        }
    }
}
