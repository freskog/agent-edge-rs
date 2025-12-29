use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    Device, FromSample, Sample, SampleFormat, SizedSample, Stream as CpalStream,
};
use crossbeam::channel::{bounded, Receiver, Sender};
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use thiserror::Error;

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
    #[error("Resampling error: {0}")]
    Resampling(String),
}

/// Audio capture configuration
#[derive(Debug, Clone)]
pub struct AudioCaptureConfig {
    /// Device ID to capture from (None = default device)
    pub device_id: Option<String>,
    /// Channel to capture (0-based index) - we always use channel 0
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

/// Audio device information
#[derive(Debug, Clone)]
pub struct AudioDeviceInfo {
    pub name: String,
    pub id: String,
    pub is_default: bool,
    pub channel_count: u32,
}

/// Sync audio capture that outputs mono 16kHz s16le chunks
/// Handles platform-specific resampling automatically
pub struct AudioCapture {
    receiver: Receiver<Vec<u8>>,
    stop_sender: Sender<()>,
    _handle: thread::JoinHandle<()>,
}

impl AudioCapture {
    /// Create a new audio capture
    /// Output is always mono 16kHz s16le regardless of hardware
    pub fn new(config: AudioCaptureConfig) -> Result<Self, AudioCaptureError> {
        let (sender, receiver) = bounded(100);
        let (stop_sender, stop_receiver) = bounded(1);

        // Spawn thread to manage CPAL resources
        let handle = thread::spawn(move || {
            if let Err(e) = Self::run_capture_thread(config, sender, stop_receiver) {
                log::error!("Audio capture thread failed: {}", e);
            }
        });

        // Give the thread a moment to initialize
        thread::sleep(Duration::from_millis(50));

        Ok(Self {
            receiver,
            stop_sender,
            _handle: handle,
        })
    }

    /// Stop the audio capture and release the device
    pub fn stop(&self) {
        let _ = self.stop_sender.send(());
    }

    /// Get the next audio chunk as s16le bytes (blocking)
    /// Returns None if the capture stream has ended
    pub fn next_chunk(&self) -> Option<Vec<u8>> {
        self.receiver.recv().ok()
    }

    /// Try to get the next audio chunk without blocking
    /// Returns None if no chunk is available or stream has ended
    pub fn try_next_chunk(&self) -> Option<Vec<u8>> {
        self.receiver.try_recv().ok()
    }

    /// Internal function that runs in the CPAL thread
    fn run_capture_thread(
        config: AudioCaptureConfig,
        sender: Sender<Vec<u8>>,
        stop_receiver: Receiver<()>,
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

        let supported_config = match Self::select_input_config(&device, config.channel) {
            Ok(config) => config,
            Err(err) => {
                log::warn!(
                    "⚠️  Failed to select preferred input config: {}. Falling back to default input config.",
                    err
                );
                device
                    .default_input_config()
                    .map_err(|e| AudioCaptureError::Config(e.to_string()))?
            }
        };

        // Verify channel selection is valid
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

        log::info!(
            "🎤 Hardware: {}Hz, {} channels, {:?} → Output: 16kHz mono s16le",
            hardware_sample_rate,
            channels,
            supported_config.sample_format()
        );

        // Create resampler if needed (macOS will need this)
        let resampler = if hardware_sample_rate != 16000 {
            let ratio = 16000.0 / hardware_sample_rate as f64;
            let params = SincInterpolationParameters {
                sinc_len: 32,
                f_cutoff: 0.95,
                interpolation: SincInterpolationType::Linear,
                oversampling_factor: 128,
                window: WindowFunction::BlackmanHarris2,
            };

            let resampler = SincFixedIn::<f32>::new(ratio, 2.0, params, CHUNK_SIZE, 1)
                .map_err(|e| AudioCaptureError::Resampling(e.to_string()))?;

            log::info!(
                "🔄 Created resampler: {}Hz → 16kHz (ratio: {:.3})",
                hardware_sample_rate,
                ratio
            );
            Some(Arc::new(Mutex::new(resampler)))
        } else {
            log::info!("🔄 No resampling needed (hardware is 16kHz)");
            None
        };

        let err_fn = move |err| {
            log::error!("Audio stream error: {}", err);
        };

        // Build the stream with the appropriate sample format
        let stream = match supported_config.sample_format() {
            SampleFormat::I16 => Self::create_input_stream::<i16>(
                &device,
                &stream_config,
                config.channel,
                channels,
                sender,
                resampler,
                err_fn,
            )?,
            SampleFormat::U16 => Self::create_input_stream::<u16>(
                &device,
                &stream_config,
                config.channel,
                channels,
                sender,
                resampler,
                err_fn,
            )?,
            SampleFormat::F32 => Self::create_input_stream::<f32>(
                &device,
                &stream_config,
                config.channel,
                channels,
                sender,
                resampler,
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
            if stop_receiver.try_recv().is_ok() {
                log::info!("Audio capture thread received stop signal. Exiting.");
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }

        Ok(())
    }

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
                SampleFormat::F32 => 1,
                SampleFormat::U16 => 2,
                _ => 3,
            };

            let min_rate = config_range.min_sample_rate().0;
            let max_rate = config_range.max_sample_rate().0;
            let target_rate = 16000;

            let chosen_rate = if target_rate < min_rate {
                min_rate
            } else if target_rate > max_rate {
                max_rate
            } else {
                target_rate
            };

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

    fn create_input_stream<T>(
        device: &Device,
        config: &cpal::StreamConfig,
        channel: u32,
        channels: usize,
        sender: Sender<Vec<u8>>,
        resampler: Option<Arc<Mutex<SincFixedIn<f32>>>>,
        err_fn: impl FnMut(cpal::StreamError) + Send + 'static + Copy,
    ) -> Result<CpalStream, AudioCaptureError>
    where
        T: Sample + SizedSample + Send + Sync + 'static,
        f32: FromSample<T>,
    {
        let mut f32_buffer = Vec::new();

        device
            .build_input_stream(
                config,
                move |data: &[T], _| {
                    // Extract channel 0 and convert to f32
                    for frame in data.chunks(channels) {
                        if let Some(s) = frame.get(channel as usize) {
                            f32_buffer.push(f32::from_sample(*s));
                        }
                    }

                    // Process complete chunks
                    while f32_buffer.len() >= CHUNK_SIZE {
                        let chunk: Vec<f32> = f32_buffer.drain(..CHUNK_SIZE).collect();

                        // Apply resampling if needed
                        let output_samples = if let Some(ref resampler) = resampler {
                            match resampler.lock() {
                                Ok(mut r) => match r.process(&[chunk], None) {
                                    Ok(output_channels) => output_channels[0].clone(),
                                    Err(e) => {
                                        log::error!("Resampling error: {}", e);
                                        continue;
                                    }
                                },
                                Err(_) => {
                                    log::error!("Failed to lock resampler");
                                    continue;
                                }
                            }
                        } else {
                            chunk
                        };

                        // Convert to s16le bytes
                        let s16le_bytes = Self::f32_to_s16le_bytes(&output_samples);

                        // Send to receiver (non-blocking)
                        if sender.try_send(s16le_bytes).is_err() {
                            // Channel is full or closed, drop the chunk
                            break;
                        }
                    }
                },
                err_fn,
                None,
            )
            .map_err(|e| AudioCaptureError::Stream(e.to_string()))
    }

    /// Convert f32 samples to s16le bytes
    fn f32_to_s16le_bytes(f32_samples: &[f32]) -> Vec<u8> {
        let mut s16le_bytes = Vec::with_capacity(f32_samples.len() * 2);

        for &sample in f32_samples {
            // Clamp to [-1.0, 1.0] and convert to i16 using symmetric scaling
            let clamped = sample.clamp(-1.0, 1.0);
            // Use 32768.0 for proper symmetric conversion (matches STT service expectation)
            let i16_sample = (clamped * 32768.0).clamp(-32768.0, 32767.0) as i16;
            s16le_bytes.extend_from_slice(&i16_sample.to_le_bytes());
        }

        s16le_bytes
    }

    /// List available audio devices
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
        // Give the thread a moment to clean up
        thread::sleep(Duration::from_millis(10));
    }
}
