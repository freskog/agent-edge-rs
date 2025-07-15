use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    Device, FromSample, Sample, SampleFormat, SizedSample, Stream as CpalStream,
};
use futures::Stream;
use ringbuf::traits::{Consumer, Producer, Split};
use ringbuf::HeapRb;
use std::pin::Pin;
use std::task::{Context, Poll};
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
    receiver: tokio_mpsc::Receiver<[f32; CHUNK_SIZE]>,
}

impl AudioCapture {
    /// Create a new audio capture with clean async interface
    pub async fn new(config: AudioCaptureConfig) -> Result<Self, AudioCaptureError> {
        let (sender, receiver) = tokio_mpsc::channel(100);

        // Spawn internal thread to manage CPAL resources
        std::thread::spawn(move || {
            if let Err(e) = Self::run_capture_thread(config, sender) {
                log::error!("Audio capture thread failed: {}", e);
            }
        });

        // Give the thread a moment to initialize
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        Ok(Self { receiver })
    }

    /// Get the next audio chunk
    pub async fn next_chunk(&mut self) -> Option<[f32; CHUNK_SIZE]> {
        self.receiver.recv().await
    }
    /// Internal function that runs in the CPAL thread
    fn run_capture_thread(
        config: AudioCaptureConfig,
        sender: tokio_mpsc::Sender<[f32; CHUNK_SIZE]>,
    ) -> Result<(), AudioCaptureError> {
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
            SampleFormat::I16 => create_input_stream::<i16>(
                &device,
                &stream_config,
                channel,
                channels,
                sender,
                err_fn,
            )?,
            SampleFormat::U16 => create_input_stream::<u16>(
                &device,
                &stream_config,
                channel,
                channels,
                sender,
                err_fn,
            )?,
            SampleFormat::F32 => create_input_stream::<f32>(
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

        // Keep the stream and host alive by holding them in this thread
        let _stream = stream;
        let _host = host;

        // Keep the thread alive to maintain the audio capture
        loop {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
}

impl Stream for AudioCapture {
    type Item = [f32; CHUNK_SIZE];

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.receiver.poll_recv(cx)
    }
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
    let rb = HeapRb::<[f32; CHUNK_SIZE]>::new(16);

    let (mut prod, mut cons) = rb.split();

    // Spawn a thread with its own runtime for the ringbuffer reader
    // This bridges the sync CPAL world with the async tokio world
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async move {
            loop {
                if let Some(chunk) = cons.try_pop() {
                    if sender.send(chunk).await.is_err() {
                        break;
                    }
                } else {
                    // No data available, sleep briefly to avoid busy-waiting
                    tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
                }
            }
        });
    });

    let mut buf = [0.0; CHUNK_SIZE];
    let mut i = 0;

    device
        .build_input_stream(
            config,
            move |data: &[T], _| {
                for frame in data.chunks(channels) {
                    if let Some(s) = frame.get(channel as usize) {
                        buf[i] = f32::from_sample(*s);
                        i += 1;
                        if i == CHUNK_SIZE {
                            let _ = prod.try_push(buf);
                            buf = [0.0; CHUNK_SIZE];
                            i = 0;
                        }
                    }
                }
            },
            err_fn,
            None,
        )
        .map_err(|e| AudioCaptureError::Stream(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_audio_capture_creation() {
        match AudioCapture::new(AudioCaptureConfig::default()).await {
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
