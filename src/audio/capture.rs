use crate::audio::ChannelExtractor;
use crate::error::{EdgeError, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, Host, SampleFormat, Stream, SupportedStreamConfig};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

/// Audio capture configuration
#[derive(Debug, Clone)]
pub struct AudioCaptureConfig {
    pub sample_rate: u32,
    pub channels: usize,
    pub device_name: Option<String>,
    pub buffer_size: usize,
    pub target_latency_ms: u32,
}

impl Default for AudioCaptureConfig {
    fn default() -> Self {
        Self {
            sample_rate: 16000, // Standard for speech processing
            channels: 6,        // ReSpeaker 4-mic array has 6 channels
            device_name: None,
            buffer_size: 1024,     // Audio buffer size
            target_latency_ms: 50, // 50ms target latency for PulseAudio
        }
    }
}

/// Audio sample data
pub type AudioSample = f32;
pub type AudioBuffer = Vec<AudioSample>;

/// Audio capture interface
pub struct AudioCapture {
    config: AudioCaptureConfig,
    host: Host,
    device: Option<Device>,
    stream: Option<Stream>,
    channel_extractor: ChannelExtractor,
    audio_receiver: Option<Receiver<AudioBuffer>>,
}

impl AudioCapture {
    pub fn new(config: AudioCaptureConfig) -> Result<Self> {
        log::info!("Initializing audio capture with config: {:?}", config);

        // Get the default host - on Linux with PulseAudio, ALSA will route through PulseAudio
        let host = cpal::default_host();

        log::info!("Using audio host: {}", host.id().name());
        #[cfg(target_os = "linux")]
        log::info!("On Linux: ALSA will route through PulseAudio when available");

        // Set up channel extractor for ReSpeaker (extract channel 0 from 6 channels)
        let channel_extractor = ChannelExtractor::new(0, config.channels)
            .map_err(|e| EdgeError::Audio(format!("Failed to create channel extractor: {}", e)))?;

        Ok(Self {
            config,
            host,
            device: None,
            stream: None,
            channel_extractor,
            audio_receiver: None,
        })
    }

    /// Get the audio input device
    fn get_input_device(&self) -> Result<Device> {
        if let Some(device_name) = &self.config.device_name {
            // Try to find device by name
            let devices = self.host.input_devices().map_err(|e| {
                EdgeError::Audio(format!("Failed to enumerate input devices: {}", e))
            })?;

            for device in devices {
                let name = device
                    .name()
                    .map_err(|e| EdgeError::Audio(format!("Failed to get device name: {}", e)))?;
                if name.contains(device_name) {
                    log::info!("Found matching device: {}", name);
                    return Ok(device);
                }
            }

            return Err(EdgeError::Audio(format!(
                "Device '{}' not found",
                device_name
            )));
        } else {
            // Use default input device
            self.host
                .default_input_device()
                .ok_or_else(|| EdgeError::Audio("No default input device available".to_string()))
        }
    }

    /// Configure the audio stream
    fn configure_stream(&self, device: &Device) -> Result<SupportedStreamConfig> {
        let supported_configs: Vec<_> = device
            .supported_input_configs()
            .map_err(|e| EdgeError::Audio(format!("Failed to get supported configs: {}", e)))?
            .collect();

        log::debug!(
            "Looking for {} channels at {} Hz",
            self.config.channels,
            self.config.sample_rate
        );

        // Log all available configs for debugging
        for config_range in &supported_configs {
            log::debug!(
                "Available config: {} channels, {}-{} Hz, format: {:?}",
                config_range.channels(),
                config_range.min_sample_rate().0,
                config_range.max_sample_rate().0,
                config_range.sample_format()
            );
        }

        // First, try to find exact match
        for config_range in &supported_configs {
            if config_range.channels() == self.config.channels as u16
                && config_range.min_sample_rate().0 <= self.config.sample_rate
                && config_range.max_sample_rate().0 >= self.config.sample_rate
            {
                let config =
                    config_range.with_sample_rate(cpal::SampleRate(self.config.sample_rate));
                log::info!("Found exact match: {:?}", config);
                return Ok(config);
            }
        }

        // If no exact match, try to find the best available configuration
        // First priority: matching channel count with any sample rate
        for config_range in &supported_configs {
            if config_range.channels() == self.config.channels as u16 {
                // Use the closest available sample rate
                let target_rate = self.config.sample_rate;
                let min_rate = config_range.min_sample_rate().0;
                let max_rate = config_range.max_sample_rate().0;

                let actual_rate = if target_rate < min_rate {
                    min_rate
                } else if target_rate > max_rate {
                    max_rate
                } else {
                    target_rate
                };

                let config = config_range.with_sample_rate(cpal::SampleRate(actual_rate));
                log::info!(
                    "Using config with sample rate adjustment: {:?} (requested {} Hz)",
                    config,
                    target_rate
                );
                return Ok(config);
            }
        }

        // Last resort: any config with matching sample rate range
        for config_range in &supported_configs {
            if config_range.min_sample_rate().0 <= self.config.sample_rate
                && config_range.max_sample_rate().0 >= self.config.sample_rate
            {
                let config =
                    config_range.with_sample_rate(cpal::SampleRate(self.config.sample_rate));
                log::info!(
                    "Using fallback config: {:?} (requested {} channels)",
                    config,
                    self.config.channels
                );
                return Ok(config);
            }
        }

        Err(EdgeError::Audio(format!(
            "No suitable audio configuration found for {} channels at {} Hz",
            self.config.channels, self.config.sample_rate
        )))
    }

    pub fn start(&mut self) -> Result<()> {
        log::info!("Starting audio capture");

        // Get the input device
        let device = self.get_input_device()?;
        let device_name = device
            .name()
            .map_err(|e| EdgeError::Audio(format!("Failed to get device name: {}", e)))?;
        log::info!("Using input device: {}", device_name);

        // Configure the stream
        let config = self.configure_stream(&device)?;

        // Create channel for sending audio data
        let (sender, receiver) = mpsc::channel();
        self.audio_receiver = Some(receiver);

        // Create the audio stream
        let stream = self.create_input_stream(&device, &config, sender)?;

        // Start the stream
        stream
            .play()
            .map_err(|e| EdgeError::Audio(format!("Failed to start audio stream: {}", e)))?;

        self.device = Some(device);
        self.stream = Some(stream);

        log::info!("Audio capture started successfully");
        Ok(())
    }

    /// Create the input stream with the specified configuration
    fn create_input_stream(
        &self,
        device: &Device,
        config: &SupportedStreamConfig,
        sender: Sender<AudioBuffer>,
    ) -> Result<Stream> {
        let config_clone = config.config();
        let _channels = self.config.channels;
        let channel_extractor = Arc::new(Mutex::new(self.channel_extractor.clone()));

        let stream = match config.sample_format() {
            SampleFormat::F32 => device
                .build_input_stream(
                    &config_clone,
                    move |data: &[f32], _: &cpal::InputCallbackInfo| {
                        Self::process_audio_callback(data, &sender, &channel_extractor);
                    },
                    |err| log::error!("Audio stream error: {}", err),
                    None,
                )
                .map_err(|e| {
                    EdgeError::Audio(format!("Failed to build f32 input stream: {}", e))
                })?,
            SampleFormat::I16 => {
                device
                    .build_input_stream(
                        &config_clone,
                        move |data: &[i16], _: &cpal::InputCallbackInfo| {
                            // Convert i16 to f32
                            let float_data: Vec<f32> = data
                                .iter()
                                .map(|&sample| sample as f32 / i16::MAX as f32)
                                .collect();
                            Self::process_audio_callback(&float_data, &sender, &channel_extractor);
                        },
                        |err| log::error!("Audio stream error: {}", err),
                        None,
                    )
                    .map_err(|e| {
                        EdgeError::Audio(format!("Failed to build i16 input stream: {}", e))
                    })?
            }
            SampleFormat::U16 => {
                device
                    .build_input_stream(
                        &config_clone,
                        move |data: &[u16], _: &cpal::InputCallbackInfo| {
                            // Convert u16 to f32
                            let float_data: Vec<f32> = data
                                .iter()
                                .map(|&sample| {
                                    (sample as f32 - u16::MAX as f32 / 2.0)
                                        / (u16::MAX as f32 / 2.0)
                                })
                                .collect();
                            Self::process_audio_callback(&float_data, &sender, &channel_extractor);
                        },
                        |err| log::error!("Audio stream error: {}", err),
                        None,
                    )
                    .map_err(|e| {
                        EdgeError::Audio(format!("Failed to build u16 input stream: {}", e))
                    })?
            }
            _ => return Err(EdgeError::Audio("Unsupported sample format".to_string())),
        };

        Ok(stream)
    }

    /// Process audio callback and extract channel 0
    fn process_audio_callback(
        data: &[f32],
        sender: &Sender<AudioBuffer>,
        channel_extractor: &Arc<Mutex<ChannelExtractor>>,
    ) {
        if let Ok(extractor) = channel_extractor.lock() {
            let channel_0_data = extractor.extract_channel(data);
            if let Err(e) = sender.send(channel_0_data) {
                log::warn!("Failed to send audio data: {}", e);
            }
        }
    }

    /// Get the next audio buffer (blocking)
    pub fn get_audio_buffer(&self) -> Result<AudioBuffer> {
        if let Some(receiver) = &self.audio_receiver {
            receiver
                .recv()
                .map_err(|e| EdgeError::Audio(format!("Failed to receive audio data: {}", e)))
        } else {
            Err(EdgeError::Audio("Audio capture not started".to_string()))
        }
    }

    /// Try to get the next audio buffer (non-blocking)
    pub fn try_get_audio_buffer(&self) -> Result<Option<AudioBuffer>> {
        if let Some(receiver) = &self.audio_receiver {
            match receiver.try_recv() {
                Ok(buffer) => Ok(Some(buffer)),
                Err(mpsc::TryRecvError::Empty) => Ok(None),
                Err(mpsc::TryRecvError::Disconnected) => {
                    Err(EdgeError::Audio("Audio stream disconnected".to_string()))
                }
            }
        } else {
            Err(EdgeError::Audio("Audio capture not started".to_string()))
        }
    }

    pub fn stop(&mut self) -> Result<()> {
        log::info!("Stopping audio capture");

        if let Some(stream) = self.stream.take() {
            drop(stream);
        }

        self.device = None;
        self.audio_receiver = None;

        log::info!("Audio capture stopped");
        Ok(())
    }

    /// List available input devices
    pub fn list_input_devices(&self) -> Result<Vec<String>> {
        let devices = self
            .host
            .input_devices()
            .map_err(|e| EdgeError::Audio(format!("Failed to enumerate input devices: {}", e)))?;

        let mut device_names = Vec::new();
        for device in devices {
            let name = device
                .name()
                .map_err(|e| EdgeError::Audio(format!("Failed to get device name: {}", e)))?;
            device_names.push(name);
        }

        Ok(device_names)
    }
}
