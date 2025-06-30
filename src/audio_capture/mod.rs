use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    Device, FromSample, Host, Sample, SampleFormat, SizedSample, Stream as CpalStream,
};
use futures_util::Stream;
use std::{
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};
use thiserror::Error;
use tokio::sync::mpsc;

const SAMPLE_RATE: u32 = 16_000;
const CHUNK_SIZE: usize = 256;

#[derive(Error, Debug)]
pub enum AudioCaptureError {
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
    /// Preferred sample format (None = use device native)
    pub preferred_format: Option<SampleFormat>,
}

impl Default for AudioCaptureConfig {
    fn default() -> Self {
        Self {
            device_id: None,
            channel: 0,
            preferred_format: None,
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

/// A chunk of audio samples in i16 format
#[derive(Clone, Debug)]
pub struct AudioChunk {
    /// Raw samples in i16 format (native for VAD)
    pub samples: Vec<i16>,
}

/// Audio capture implementation using CPAL
pub struct CpalAudioCapture {
    config: AudioCaptureConfig,
    stream: Option<CpalStream>,
    rx: mpsc::Receiver<AudioChunk>,
    _host: Host,
}

impl CpalAudioCapture {
    pub fn new(config: AudioCaptureConfig) -> Result<Self, AudioCaptureError> {
        let host = cpal::default_host();

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

        // Create channel for audio data
        let (tx, rx) = mpsc::channel(32);
        let tx = Arc::new(Mutex::new(tx));

        // Get supported configs
        let supported_configs: Vec<_> = device
            .supported_input_configs()
            .map_err(|e| AudioCaptureError::Config(e.to_string()))?
            .collect();

        // Try to find a config that supports our required sample rate
        let mut supported_config = None;

        // First, try to find a config that natively supports 16kHz
        for config in &supported_configs {
            if config.min_sample_rate().0 <= SAMPLE_RATE
                && config.max_sample_rate().0 >= SAMPLE_RATE
            {
                supported_config = Some(config.with_sample_rate(cpal::SampleRate(SAMPLE_RATE)));
                log::info!(
                    "Found config with native 16kHz support: {:?}",
                    config.sample_format()
                );
                break;
            }
        }

        // If no native 16kHz support, use default config and let CPAL handle resampling
        if supported_config.is_none() {
            supported_config = Some(
                device
                    .default_input_config()
                    .map_err(|e| AudioCaptureError::Config(e.to_string()))?,
            );
            log::info!(
                "Using default config with resampling: {:?} @ {}Hz",
                supported_config.as_ref().unwrap().sample_format(),
                supported_config.as_ref().unwrap().sample_rate().0
            );
        }

        let supported_config = supported_config.unwrap();

        // Verify channel selection is valid
        if config.channel >= u32::from(supported_config.channels()) {
            return Err(AudioCaptureError::Config(format!(
                "Selected channel {} is not available (device has {} channels)",
                config.channel,
                supported_config.channels()
            )));
        }

        // Create a config with our required sample rate
        let stream_config = cpal::StreamConfig {
            channels: supported_config.channels(),
            sample_rate: cpal::SampleRate(SAMPLE_RATE),
            buffer_size: cpal::BufferSize::Default,
        };

        let err_fn = move |err| {
            log::error!("Audio stream error: {}", err);
        };

        // Log the format being used
        log::info!(
            "Audio capture configured: {} channels @ {}Hz (format: {:?})",
            stream_config.channels,
            SAMPLE_RATE,
            supported_config.sample_format()
        );

        // Create the stream using the device's native format
        let stream = match supported_config.sample_format() {
            SampleFormat::I16 => Self::build_stream::<i16>(
                &device,
                &stream_config,
                tx.clone(),
                config.channel,
                err_fn,
            )?,
            SampleFormat::U16 => Self::build_stream::<u16>(
                &device,
                &stream_config,
                tx.clone(),
                config.channel,
                err_fn,
            )?,
            SampleFormat::F32 => Self::build_stream::<f32>(
                &device,
                &stream_config,
                tx.clone(),
                config.channel,
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
            config,
            stream: Some(stream),
            rx,
            _host: host,
        })
    }

    fn build_stream<T>(
        device: &Device,
        config: &cpal::StreamConfig,
        tx: Arc<Mutex<mpsc::Sender<AudioChunk>>>,
        channel: u32,
        err_fn: impl FnMut(cpal::StreamError) + Send + 'static + Copy,
    ) -> Result<CpalStream, AudioCaptureError>
    where
        T: Sample + SizedSample + Send + Sync + 'static,
        i16: FromSample<T>,
    {
        let mut buffer = Vec::with_capacity(CHUNK_SIZE);
        let channels = config.channels as usize;

        device
            .build_input_stream(
                config,
                move |data: &[T], _: &cpal::InputCallbackInfo| {
                    // Extract the specified channel and convert to i16
                    for frame in data.chunks(channels) {
                        if let Some(sample) = frame.get(channel as usize) {
                            // Convert to i16 using CPAL's conversion trait
                            let value = i16::from_sample(*sample);
                            buffer.push(value);

                            if buffer.len() >= CHUNK_SIZE {
                                if let Ok(tx) = tx.lock() {
                                    let chunk = AudioChunk {
                                        samples: buffer.clone(),
                                    };
                                    let _ = tx.try_send(chunk);
                                }
                                buffer.clear();
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

impl Stream for CpalAudioCapture {
    type Item = Result<AudioChunk, AudioCaptureError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.rx.poll_recv(cx) {
            Poll::Ready(Some(chunk)) => Poll::Ready(Some(Ok(chunk))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}
