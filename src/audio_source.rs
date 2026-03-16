use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    Device, SampleFormat, Stream as CpalStream,
};
use crossbeam::channel::{bounded, Receiver, Sender};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use thiserror::Error;

pub use crate::types::AudioDeviceInfo;

use std::time::Instant;

pub const CHUNK_SIZE: usize = 1280; // Fixed chunk size (in samples)

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
}

impl Default for AudioCaptureConfig {
    fn default() -> Self {
        Self {
            device_id: None,
            channel: 0,
        }
    }
}

/// Sync audio capture that outputs mono 16kHz s16le chunks.
/// Assumes hardware delivers I16 at 16kHz (XVF3800).
pub struct AudioCapture {
    receiver: Receiver<Vec<u8>>,
    stop_sender: Sender<()>,
    _handle: thread::JoinHandle<()>,
}

impl AudioCapture {
    pub fn new(config: AudioCaptureConfig) -> Result<Self, AudioCaptureError> {
        let (sender, receiver) = bounded(15);
        let (stop_sender, stop_receiver) = bounded(1);

        let handle = thread::spawn(move || {
            if let Err(e) = Self::run_capture_thread(config, sender, stop_receiver) {
                log::error!("Audio capture thread failed: {}", e);
            }
        });

        thread::sleep(Duration::from_millis(50));

        Ok(Self {
            receiver,
            stop_sender,
            _handle: handle,
        })
    }

    pub fn stop(&self) {
        let _ = self.stop_sender.send(());
    }

    /// Get the next audio chunk as s16le bytes (blocking).
    pub fn next_chunk(&self) -> Option<Vec<u8>> {
        self.receiver.recv().ok()
    }

    /// Try to get the next audio chunk without blocking.
    pub fn try_next_chunk(&self) -> Option<Vec<u8>> {
        self.receiver.try_recv().ok()
    }

    fn run_capture_thread(
        config: AudioCaptureConfig,
        sender: Sender<Vec<u8>>,
        stop_receiver: Receiver<()>,
    ) -> Result<(), AudioCaptureError> {
        let host = cpal::default_host();
        log::info!("🎤 Initializing audio capture with host: {:?}", host.id());

        let device = if let Some(id) = &config.device_id {
            if id == "default" {
                host.default_input_device().ok_or_else(|| {
                    AudioCaptureError::Device("No default input device found".into())
                })?
            } else {
                host.devices()
                    .map_err(|e| AudioCaptureError::Device(e.to_string()))?
                    .find(|d| d.name().map(|n| n == *id).unwrap_or(false))
                    .ok_or_else(|| {
                        AudioCaptureError::Device(format!("Device not found: {}", id))
                    })?
            }
        } else {
            host.default_input_device()
                .ok_or_else(|| AudioCaptureError::Device("No default input device found".into()))?
        };

        log::info!("🎤 Using input device: {:?}", device.name());

        let stream_broken = Arc::new(AtomicBool::new(false));

        let (mut stream, _hardware_sample_rate) =
            Self::try_open_stream(&device, &config, &sender, Arc::clone(&stream_broken))?;

        stream
            .play()
            .map_err(|e| AudioCaptureError::Stream(e.to_string()))?;

        let _host = host;
        let mut last_recreation = Instant::now();

        loop {
            if stop_receiver.try_recv().is_ok() {
                log::info!("Audio capture thread received stop signal. Exiting.");
                break;
            }

            if stream_broken.load(Ordering::Acquire)
                && last_recreation.elapsed() >= Duration::from_millis(500)
            {
                log::warn!("Capture stream broken (xrun), recreating...");
                drop(stream);
                stream_broken.store(false, Ordering::Release);

                match Self::try_open_stream(
                    &device,
                    &config,
                    &sender,
                    Arc::clone(&stream_broken),
                ) {
                    Ok((new_stream, _)) => {
                        if let Err(e) = new_stream.play() {
                            log::error!("Failed to restart capture stream: {}", e);
                            break;
                        }
                        stream = new_stream;
                        last_recreation = Instant::now();
                        log::info!("Capture stream recreated successfully after xrun");
                    }
                    Err(e) => {
                        log::error!("Failed to recreate capture stream: {}", e);
                        break;
                    }
                }
            }

            thread::sleep(Duration::from_millis(100));
        }

        Ok(())
    }

    fn try_open_stream(
        device: &Device,
        config: &AudioCaptureConfig,
        sender: &Sender<Vec<u8>>,
        stream_broken: Arc<AtomicBool>,
    ) -> Result<(CpalStream, u32), AudioCaptureError> {
        let supported_config = match Self::select_input_config(device, config.channel) {
            Ok(cfg) => cfg,
            Err(err) => {
                log::warn!(
                    "⚠️  Failed to select preferred input config: {}. Falling back to default.",
                    err
                );
                device
                    .default_input_config()
                    .map_err(|e| AudioCaptureError::Config(e.to_string()))?
            }
        };

        if config.channel >= u32::from(supported_config.channels()) {
            return Err(AudioCaptureError::Config(format!(
                "Selected channel {} is not available (device has {} channels)",
                config.channel,
                supported_config.channels()
            )));
        }

        let stream_config = supported_config.config();
        let hardware_sample_rate = stream_config.sample_rate.0;
        let channels = stream_config.channels as usize;

        if supported_config.sample_format() != SampleFormat::I16 {
            return Err(AudioCaptureError::Config(format!(
                "Expected I16 sample format, got {:?}",
                supported_config.sample_format()
            )));
        }
        if hardware_sample_rate != 16000 {
            return Err(AudioCaptureError::Config(format!(
                "Expected 16kHz sample rate, got {}Hz",
                hardware_sample_rate
            )));
        }

        log::info!(
            "🎤 Hardware: {}Hz, {} channels, I16 → Output: 16kHz mono s16le",
            hardware_sample_rate,
            channels,
        );

        let sender = sender.clone();
        let stream = Self::create_native_i16_stream(
            device,
            &stream_config,
            config.channel,
            channels,
            sender,
            stream_broken,
        )?;

        Ok((stream, hardware_sample_rate))
    }

    /// Prefer I16 at 16kHz; reject anything else.
    fn select_input_config(
        device: &Device,
        channel: u32,
    ) -> Result<cpal::SupportedStreamConfig, AudioCaptureError> {
        let configs = device
            .supported_input_configs()
            .map_err(|e| AudioCaptureError::Config(e.to_string()))?;

        let mut best_config: Option<cpal::SupportedStreamConfig> = None;
        let mut best_format_rank = u8::MAX;
        let mut best_rate_diff = u32::MAX;

        for config_range in configs {
            let channels = config_range.channels() as u32;
            if channel >= channels {
                continue;
            }

            let format_rank = match config_range.sample_format() {
                SampleFormat::I16 => 0,
                _ => 3,
            };

            let min_rate = config_range.min_sample_rate().0;
            let max_rate = config_range.max_sample_rate().0;
            let target_rate = 16000;

            let chosen_rate = target_rate.clamp(min_rate, max_rate);
            let rate_diff = chosen_rate.abs_diff(target_rate);
            let config = config_range.with_sample_rate(cpal::SampleRate(chosen_rate));

            if format_rank < best_format_rank
                || (format_rank == best_format_rank && rate_diff < best_rate_diff)
            {
                best_format_rank = format_rank;
                best_rate_diff = rate_diff;
                best_config = Some(config);
            }
        }

        best_config.ok_or_else(|| {
            AudioCaptureError::Config("No supported input configs found".to_string())
        })
    }

    /// Allocation-free I16 capture path.
    /// Pre-allocates all buffers; the only allocation per chunk is the Vec<u8>
    /// sent over the channel (~12.5Hz), not per-sample or per-callback.
    fn create_native_i16_stream(
        device: &Device,
        config: &cpal::StreamConfig,
        channel: u32,
        channels: usize,
        sender: Sender<Vec<u8>>,
        stream_broken: Arc<AtomicBool>,
    ) -> Result<CpalStream, AudioCaptureError> {
        let chunk_bytes = CHUNK_SIZE * 2;
        let mut byte_buffer: Vec<u8> = Vec::with_capacity(chunk_bytes * 8);
        let mut chunk_buf: Vec<u8> = vec![0u8; chunk_bytes];

        device
            .build_input_stream(
                config,
                move |data: &[i16], _| {
                    for frame in data.chunks(channels) {
                        if let Some(&s) = frame.get(channel as usize) {
                            byte_buffer.extend_from_slice(&s.to_le_bytes());
                        }
                    }

                    let mut read_pos = 0;
                    while read_pos + chunk_bytes <= byte_buffer.len() {
                        chunk_buf
                            .copy_from_slice(&byte_buffer[read_pos..read_pos + chunk_bytes]);
                        if sender.try_send(chunk_buf.clone()).is_err() {
                            read_pos += chunk_bytes;
                            break;
                        }
                        read_pos += chunk_bytes;
                    }

                    if read_pos > 0 {
                        byte_buffer.drain(..read_pos);
                    }
                },
                move |err| {
                    if stream_broken
                        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
                        .is_ok()
                    {
                        log::error!("Audio stream error: {}", err);
                    }
                },
                None,
            )
            .map_err(|e| AudioCaptureError::Stream(e.to_string()))
    }

    pub fn list_devices() -> Result<Vec<AudioDeviceInfo>, AudioCaptureError> {
        let host = cpal::default_host();
        let devices = host
            .input_devices()
            .map_err(|e| AudioCaptureError::Device(e.to_string()))?;

        let default_device = host.default_input_device();
        let default_name = default_device.and_then(|d| d.name().ok());

        let mut device_infos = Vec::new();
        for device in devices {
            let name = device
                .name()
                .map_err(|e| AudioCaptureError::Device(e.to_string()))?;

            let config = device
                .default_input_config()
                .map_err(|e| AudioCaptureError::Config(e.to_string()))?;

            device_infos.push(AudioDeviceInfo {
                id: name.clone(),
                name: name.clone(),
                is_default: default_name.as_ref() == Some(&name),
                channel_count: config.channels() as u32,
            });
        }

        Ok(device_infos)
    }
}

impl Drop for AudioCapture {
    fn drop(&mut self) {
        log::debug!("🎤 Dropping AudioCapture - sending stop signal");
        let _ = self.stop_sender.send(());
        thread::sleep(Duration::from_millis(10));
    }
}
