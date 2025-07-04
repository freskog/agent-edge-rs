use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{
    BuildStreamError, DeviceNameError, DevicesError, PlayStreamError, SupportedStreamConfigsError,
};
use log::error;
use rubato::{SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;
use thiserror::Error;

#[derive(Error, Debug, Clone)]
pub enum AudioError {
    #[error("Failed to write audio data: {0}")]
    WriteError(String),

    #[error("Failed to stop audio playback: {0}")]
    StopError(String),

    #[error("Buffer full")]
    BufferFull,

    #[error("Failed to create WAV file: {0}")]
    WavCreationError(String),

    #[error("MP3 decoding not implemented")]
    Mp3DecodingNotImplemented,

    #[error("Base64 decode error: {0}")]
    Base64DecodeError(String),

    #[error("Invalid JSON: {0}")]
    InvalidJson(String),

    #[error("Missing field: {0}")]
    MissingField(String),

    #[error("Failed to save audio: {0}")]
    FailedToSaveAudio(String),

    #[error("Audio device error: {0}")]
    DeviceError(String),
}

impl From<SupportedStreamConfigsError> for AudioError {
    fn from(err: SupportedStreamConfigsError) -> Self {
        AudioError::DeviceError(err.to_string())
    }
}

impl From<BuildStreamError> for AudioError {
    fn from(err: BuildStreamError) -> Self {
        AudioError::DeviceError(err.to_string())
    }
}

impl From<PlayStreamError> for AudioError {
    fn from(err: PlayStreamError) -> Self {
        AudioError::DeviceError(err.to_string())
    }
}

impl From<DevicesError> for AudioError {
    fn from(err: DevicesError) -> Self {
        AudioError::DeviceError(err.to_string())
    }
}

impl From<DeviceNameError> for AudioError {
    fn from(err: DeviceNameError) -> Self {
        AudioError::DeviceError(err.to_string())
    }
}

/// Core trait for audio output handling
#[async_trait::async_trait]
pub trait AudioSink: Send + Sync {
    /// Write audio data to the sink. The data is expected to be
    /// 16-bit PCM at 16kHz mono.
    async fn write(&self, audio_data: &[u8]) -> Result<(), AudioError>;

    /// Stop audio playback and clear any buffered data
    async fn stop(&self) -> Result<(), AudioError>;
}

pub struct CpalConfig {
    /// Buffer size in milliseconds (default 45000ms)
    pub buffer_size_ms: u32,
    /// Warning threshold for low buffer (percentage)
    pub low_buffer_warning: u8,
    /// Warning threshold for high buffer_warning (percentage)
    pub high_buffer_warning: u8,
    /// Optional output device name
    pub device_name: Option<String>,
}

impl Default for CpalConfig {
    fn default() -> Self {
        Self {
            buffer_size_ms: 45000, // 45 seconds buffer
            low_buffer_warning: 20,
            high_buffer_warning: 80,
            device_name: None,
        }
    }
}

struct CpalStats {
    buffer_samples: AtomicUsize,
    max_buffer_samples: usize,
    last_write: Mutex<Instant>,
    write_interval_ms: AtomicUsize,
}

impl CpalStats {
    fn new(max_buffer_samples: usize) -> Self {
        Self {
            buffer_samples: AtomicUsize::new(0),
            max_buffer_samples,
            last_write: Mutex::new(Instant::now()),
            write_interval_ms: AtomicUsize::new(0),
        }
    }

    fn buffer_percentage(&self) -> u8 {
        ((self.buffer_samples.load(Ordering::Acquire) * 100) / self.max_buffer_samples) as u8
    }

    fn update_buffer_size(&self, num_samples: usize) {
        self.buffer_samples.store(num_samples, Ordering::Release);
    }
}

enum AudioCommand {
    PlayAudio(Vec<f32>),
    Stop,
}

struct AudioState {
    samples_queue: Arc<Mutex<Vec<f32>>>,
    stats: Arc<CpalStats>,
    is_stopped: Arc<AtomicBool>,
    test_tone_complete: Arc<AtomicBool>,
}

pub struct CpalSink {
    audio_sender: Sender<AudioCommand>,
    stats: Arc<CpalStats>,
    config: CpalConfig,
    is_stopped: Arc<AtomicBool>,
    test_tone_complete: Arc<AtomicBool>,
    audio_thread: Option<thread::JoinHandle<()>>,
    resampler: Arc<Mutex<SincFixedIn<f32>>>,
}

impl CpalSink {
    pub fn new(config: CpalConfig) -> Result<Self, AudioError> {
        let stats = Arc::new(CpalStats::new(
            (config.buffer_size_ms as usize * 16000) / 1000,
        ));
        let stats_clone = Arc::clone(&stats);
        let is_stopped = Arc::new(AtomicBool::new(false));
        let test_tone_complete = Arc::new(AtomicBool::new(false));

        let host = cpal::default_host();

        // Get the device
        let device = if let Some(name) = &config.device_name {
            // List all available output devices
            log::info!("AudioSink: Available output devices:");
            let mut found_device = None;
            for device in host.output_devices()? {
                let device_name = device.name()?;
                log::info!("  - {}", device_name);
                if device_name == *name {
                    found_device = Some(device);
                }
            }
            found_device.ok_or_else(|| {
                AudioError::DeviceError(format!("Output device '{}' not found", name))
            })?
        } else {
            host.default_output_device()
                .ok_or_else(|| AudioError::DeviceError("No output device available".to_string()))?
        };

        log::info!("AudioSink: Using output device: {:?}", device.name());

        let mut supported_configs_range = device.supported_output_configs().map_err(|e| {
            AudioError::DeviceError(format!("Error getting supported configs: {}", e))
        })?;

        // Log all supported configurations for debugging
        log::info!("AudioSink: Available output configurations:");
        let supported_configs: Vec<_> = supported_configs_range.by_ref().collect();
        for config in supported_configs.iter() {
            log::info!(
                "  - Channels: {}, Sample rates: {} - {} Hz, Format: {:?}",
                config.channels(),
                config.min_sample_rate().0,
                config.max_sample_rate().0,
                config.sample_format()
            );
        }

        // Find a supported configuration - prefer common audio sample rates
        let supported_config = supported_configs
            .iter()
            .find(|config| {
                // Prefer mono (1 channel) configurations that support standard audio rates
                config.channels() == 1
                    && config.min_sample_rate().0 <= 44100
                    && config.max_sample_rate().0 >= 44100
            })
            .or_else(|| {
                // Fallback: any configuration that supports 44100Hz
                supported_configs.iter().find(|config| {
                    config.min_sample_rate().0 <= 44100 && config.max_sample_rate().0 >= 44100
                })
            })
            .or_else(|| {
                // Last resort: any configuration that supports 16000Hz (our TTS rate)
                supported_configs.iter().find(|config| {
                    config.min_sample_rate().0 <= 16000 && config.max_sample_rate().0 >= 16000
                })
            })
            .ok_or_else(|| {
                AudioError::DeviceError("No suitable output config found".to_string())
            })?;

        // Use 44100Hz if supported, otherwise 48000Hz, otherwise 16000Hz
        let desired_sample_rate = if supported_config.min_sample_rate().0 <= 44100
            && supported_config.max_sample_rate().0 >= 44100
        {
            cpal::SampleRate(44100)
        } else if supported_config.min_sample_rate().0 <= 48000
            && supported_config.max_sample_rate().0 >= 48000
        {
            cpal::SampleRate(48000)
        } else {
            cpal::SampleRate(16000)
        };

        let stream_config = supported_config
            .with_sample_rate(desired_sample_rate)
            .config();

        log::info!(
            "AudioSink: Selected output config - channels: {}, sample_rate: {}Hz, format: {:?}",
            stream_config.channels,
            stream_config.sample_rate.0,
            supported_config.sample_format()
        );

        let output_sample_rate = stream_config.sample_rate.0;
        let input_sample_rate = 16000; // TTS input sample rate
        let channels = stream_config.channels as usize;
        let input_buffer_size = 1024;

        let (tx, rx) = channel::<AudioCommand>();
        let samples_queue = Arc::new(Mutex::new(Vec::new()));
        let samples_queue_clone = Arc::clone(&samples_queue);
        let resampler = Arc::new(Mutex::new(
            SincFixedIn::<f32>::new(
                output_sample_rate as f64 / input_sample_rate as f64,
                2.0,
                SincInterpolationParameters {
                    sinc_len: 256,
                    f_cutoff: 0.95,
                    interpolation: SincInterpolationType::Linear,
                    oversampling_factor: 256,
                    window: WindowFunction::BlackmanHarris2,
                },
                input_buffer_size,
                channels,
            )
            .unwrap(),
        ));

        // Add a test tone to verify audio output
        {
            let mut queue = samples_queue.lock().unwrap();
            // Generate 1 second of 440Hz sine wave
            let frequency = 440.0;
            let duration = 1.0;
            let num_samples = (output_sample_rate as f32 * duration) as usize;
            let mut test_samples = Vec::with_capacity(num_samples);

            for i in 0..num_samples {
                let t = i as f32 / output_sample_rate as f32;
                let value = (2.0 * std::f32::consts::PI * frequency * t).sin() * 0.5; // 50% volume
                test_samples.push(value);
            }

            log::debug!(
                "AudioSink: Added {} test tone samples to queue",
                test_samples.len()
            );
            queue.extend(test_samples);
        }

        let audio_state = AudioState {
            samples_queue: samples_queue_clone,
            stats: Arc::clone(&stats),
            is_stopped: Arc::clone(&is_stopped),
            test_tone_complete: Arc::clone(&test_tone_complete),
        };

        // Create and start the stream in a separate thread to avoid Send/Sync issues
        let (stream_ready_tx, stream_ready_rx) =
            std::sync::mpsc::channel::<Result<(), AudioError>>();
        let stream_builder = thread::spawn(move || -> Result<(), AudioError> {
            let stream_result = device.build_output_stream(
                &stream_config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    let start = Instant::now();
                    let mut queue = audio_state.samples_queue.lock().unwrap();

                    // Fill the output buffer with zeros if we have no data
                    if queue.is_empty() {
                        for sample in data.iter_mut() {
                            *sample = 0.0;
                        }
                        audio_state.stats.update_buffer_size(0);
                        return;
                    }

                    // Copy data from our queue to the output buffer
                    let mut i = 0;
                    while i < data.len() && !queue.is_empty() {
                        data[i] = queue.remove(0);
                        i += 1;
                    }

                    // Fill any remaining space with zeros
                    for j in i..data.len() {
                        data[j] = 0.0;
                    }

                    audio_state.stats.update_buffer_size(queue.len());
                    let _elapsed = start.elapsed();
                },
                move |err| {
                    error!("Audio output error: {}", err);
                },
                None,
            );

            match stream_result {
                Ok(stream) => {
                    log::debug!("AudioSink: Stream built successfully");
                    match stream.play() {
                        Ok(_) => {
                            log::debug!("AudioSink: Stream started playing successfully");
                            // Signal that initialization was successful
                            let _ = stream_ready_tx.send(Ok(()));
                            // Keep the stream alive until the thread exits
                            std::thread::park();
                            Ok(())
                        }
                        Err(e) => {
                            let error = AudioError::from(e);
                            let _ = stream_ready_tx.send(Err(error.clone()));
                            Err(error)
                        }
                    }
                }
                Err(e) => {
                    let error = AudioError::from(e);
                    let _ = stream_ready_tx.send(Err(error.clone()));
                    Err(error)
                }
            }
        });

        // Wait for stream initialization to complete, but don't wait for the thread to finish
        match stream_ready_rx.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(Ok(())) => {
                // Stream initialized successfully
            }
            Ok(Err(e)) => {
                // Stream initialization failed
                return Err(e);
            }
            Err(_) => {
                // Timeout waiting for stream initialization
                return Err(AudioError::DeviceError(
                    "Timeout waiting for audio stream initialization".to_string(),
                ));
            }
        }

        // Start audio processing thread
        let audio_thread = thread::spawn(move || {
            log::debug!("AudioSink: Audio processing thread started");
            while let Ok(command) = rx.recv() {
                match command {
                    AudioCommand::PlayAudio(mut new_samples) => {
                        // Add samples to the queue
                        let new_len = {
                            let mut samples_queue = samples_queue.lock().unwrap();
                            let old_len = samples_queue.len();
                            samples_queue.append(&mut new_samples);
                            samples_queue.len() - old_len
                        };
                        log::debug!(
                            "AudioSink: Added {} samples to queue (total: {})",
                            new_len,
                            samples_queue.lock().unwrap().len()
                        );
                    }
                    AudioCommand::Stop => {
                        log::debug!("AudioSink: Received stop command, clearing queue");
                        samples_queue.lock().unwrap().clear();
                        break;
                    }
                }
            }

            log::debug!("AudioSink: Audio processing thread stopped");
        });

        Ok(Self {
            audio_sender: tx,
            stats: stats_clone,
            config,
            is_stopped: Arc::clone(&is_stopped),
            test_tone_complete: Arc::clone(&test_tone_complete),
            audio_thread: Some(audio_thread),
            resampler: Arc::clone(&resampler),
        })
    }

    pub fn get_stats(&self) -> (u8, usize) {
        let buffer_percentage = self.stats.buffer_percentage();
        let write_interval = self.stats.write_interval_ms.load(Ordering::Acquire);
        (buffer_percentage, write_interval)
    }
}

impl Drop for CpalSink {
    fn drop(&mut self) {
        log::debug!("AudioSink: Dropping CpalSink");
        self.is_stopped.store(true, Ordering::Release);
        if let Some(thread) = self.audio_thread.take() {
            if let Err(e) = thread.join() {
                log::error!("AudioSink: Error joining audio thread: {:?}", e);
            }
        }
    }
}

#[async_trait::async_trait]
impl AudioSink for CpalSink {
    async fn write(&self, audio_data: &[u8]) -> Result<(), AudioError> {
        if self.is_stopped.load(Ordering::Relaxed) {
            return Err(AudioError::WriteError("Audio sink is stopped".to_string()));
        }

        // Convert input bytes to f32 samples (assuming 16-bit PCM)
        let mut input_samples = Vec::with_capacity(audio_data.len() / 2);
        for chunk in audio_data.chunks_exact(2) {
            let sample = i16::from_le_bytes([chunk[0], chunk[1]]) as f32 / 32768.0;
            input_samples.push(sample);
        }

        // Get current stats
        let buffer_percentage = self.stats.buffer_percentage();

        // Check if buffer is too full
        if buffer_percentage >= self.config.high_buffer_warning {
            return Err(AudioError::BufferFull);
        }

        // Send samples to audio thread
        self.audio_sender
            .send(AudioCommand::PlayAudio(input_samples))
            .map_err(|e| AudioError::WriteError(e.to_string()))?;

        Ok(())
    }

    async fn stop(&self) -> Result<(), AudioError> {
        log::debug!("AudioSink: Stopping playback");
        self.is_stopped.store(true, Ordering::Release);
        self.audio_sender
            .send(AudioCommand::Stop)
            .map_err(|e| AudioError::StopError(e.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg_attr(not(feature = "test-audio"), ignore)]
    #[tokio::test]
    async fn test_cpal_sink_creation() -> Result<(), AudioError> {
        let config = CpalConfig::default();
        let _sink = CpalSink::new(config)?;
        Ok(())
    }

    #[cfg_attr(not(feature = "test-audio"), ignore)]
    #[tokio::test]
    async fn test_cpal_sink_write() -> Result<(), AudioError> {
        let config = CpalConfig::default();
        let sink = CpalSink::new(config)?;

        // Generate 1 second of silence
        let mut audio_data = Vec::new();
        for _ in 0..16000 {
            audio_data.extend_from_slice(&[0u8, 0u8]); // 16-bit PCM silence
        }

        sink.write(&audio_data).await?;
        Ok(())
    }

    #[cfg_attr(not(feature = "test-audio"), ignore)]
    #[tokio::test]
    async fn test_cpal_sink_stop() -> Result<(), AudioError> {
        let config = CpalConfig::default();
        let sink = CpalSink::new(config)?;
        sink.stop().await?;
        Ok(())
    }
}
