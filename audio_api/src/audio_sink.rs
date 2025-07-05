use crate::tonic::service::audio::{AudioFormat, SampleFormat};
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
use std::time::{Duration, Instant};
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
    /// in the format returned by get_format().
    async fn write(&self, audio_data: &[u8]) -> Result<(), AudioError>;

    /// Stop audio playback and clear any buffered data
    async fn stop(&self) -> Result<(), AudioError>;

    /// Get the audio format that this sink expects
    fn get_format(&self) -> AudioFormat;

    /// Signal that no more audio will be sent - used for proper completion detection
    async fn signal_end_of_stream(&self) -> Result<(), AudioError>;

    /// Wait for all queued audio to finish playing
    async fn wait_for_completion(&self) -> Result<(), AudioError>;

    /// Check if the sink is experiencing backpressure (buffer getting full)
    fn is_backpressure_active(&self) -> bool;

    /// Get current buffer utilization percentage (0-100)
    fn get_buffer_percentage(&self) -> u8;
}

#[derive(Clone)]
pub struct CpalConfig {
    /// Initial buffer size in milliseconds
    pub buffer_size_ms: u32,
    /// Maximum buffer size in milliseconds (for dynamic growth)
    pub max_buffer_size_ms: u32,
    /// Buffer size to grow by when full (milliseconds)
    pub buffer_growth_ms: u32,
    /// Warning threshold for low buffer (percentage)
    pub low_buffer_warning: u8,
    /// Warning threshold for high buffer_warning (percentage)
    pub high_buffer_warning: u8,
    /// Backpressure threshold (percentage) - when to start slowing down clients
    pub backpressure_threshold: u8,
    /// Optional output device name
    pub device_name: Option<String>,
}

impl Default for CpalConfig {
    fn default() -> Self {
        Self {
            buffer_size_ms: 10000, // 10 seconds buffer to handle larger resampled audio streams
            max_buffer_size_ms: 60000, // 60 seconds max buffer
            buffer_growth_ms: 10000, // Grow by 10 seconds when full
            low_buffer_warning: 20,
            high_buffer_warning: 80,
            backpressure_threshold: 90, // Start backpressure when buffer is 90% full
            device_name: None,
        }
    }
}

struct CpalStats {
    buffer_samples: AtomicUsize,
    max_buffer_samples: AtomicUsize, // Make this atomic for dynamic resizing
    write_interval_ms: AtomicUsize,
    end_of_stream_signaled: AtomicBool,
    // Configuration for dynamic buffer management
    output_sample_rate: u32,
    backpressure_threshold: u8,
    max_buffer_size_ms: u32,
    buffer_growth_ms: u32,
}

impl CpalStats {
    fn new(initial_buffer_samples: usize, output_sample_rate: u32, config: &CpalConfig) -> Self {
        Self {
            buffer_samples: AtomicUsize::new(0),
            max_buffer_samples: AtomicUsize::new(initial_buffer_samples),
            write_interval_ms: AtomicUsize::new(0),
            end_of_stream_signaled: AtomicBool::new(false),
            output_sample_rate,
            backpressure_threshold: config.backpressure_threshold,
            max_buffer_size_ms: config.max_buffer_size_ms,
            buffer_growth_ms: config.buffer_growth_ms,
        }
    }

    fn buffer_percentage(&self) -> u8 {
        let current = self.buffer_samples.load(Ordering::Acquire);
        let max = self.max_buffer_samples.load(Ordering::Acquire);
        if max == 0 {
            0
        } else {
            ((current * 100) / max) as u8
        }
    }

    fn should_apply_backpressure(&self) -> bool {
        self.buffer_percentage() >= self.backpressure_threshold
    }

    fn try_grow_buffer(&self) -> bool {
        let current_max = self.max_buffer_samples.load(Ordering::Acquire);
        let current_max_ms = (current_max * 1000) / self.output_sample_rate as usize;

        if current_max_ms + self.buffer_growth_ms as usize <= self.max_buffer_size_ms as usize {
            let new_max_samples = current_max
                + (self.buffer_growth_ms as usize * self.output_sample_rate as usize) / 1000;

            self.max_buffer_samples
                .store(new_max_samples, Ordering::Release);
            log::info!(
                "AudioSink: Buffer grown from {} to {} samples ({} to {}ms)",
                current_max,
                new_max_samples,
                current_max_ms,
                current_max_ms + self.buffer_growth_ms as usize
            );
            true
        } else {
            log::warn!(
                "AudioSink: Cannot grow buffer further - already at maximum size ({}ms)",
                self.max_buffer_size_ms
            );
            false
        }
    }

    fn update_buffer_size(&self, num_samples: usize) {
        self.buffer_samples.store(num_samples, Ordering::Release);
    }

    fn get_max_buffer_samples(&self) -> usize {
        self.max_buffer_samples.load(Ordering::Acquire)
    }
}

enum AudioCommand {
    PlayAudio(Vec<f32>),
    EndOfStream, // Signal that no more audio will be sent
    Stop,
}

struct AudioState {
    samples_queue: Arc<Mutex<Vec<f32>>>,
    stats: Arc<CpalStats>,
}

pub struct CpalSink {
    audio_sender: std::sync::mpsc::SyncSender<AudioCommand>,
    stats: Arc<CpalStats>,
    config: CpalConfig,
    is_stopped: Arc<AtomicBool>,
    audio_thread: Option<thread::JoinHandle<()>>,
    // Store the selected format
    selected_format: AudioFormat,
}

impl CpalSink {
    pub fn new(config: CpalConfig) -> Result<Self, AudioError> {
        let stats_clone;
        let is_stopped = Arc::new(AtomicBool::new(false));
        let is_stopped_for_stream = Arc::clone(&is_stopped);
        let is_stopped_for_struct = Arc::clone(&is_stopped);

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
        let output_channels = stream_config.channels;

        // Convert CPAL sample format to gRPC SampleFormat
        let sample_format = match supported_config.sample_format() {
            cpal::SampleFormat::I16 => SampleFormat::I16,
            cpal::SampleFormat::F32 => SampleFormat::F32,
            cpal::SampleFormat::I24 => SampleFormat::I24,
            cpal::SampleFormat::I32 => SampleFormat::I32,
            cpal::SampleFormat::F64 => SampleFormat::F64,
            _ => {
                log::warn!("Unsupported CPAL sample format, defaulting to F32");
                SampleFormat::F32
            }
        };

        // Create audio processing channel with reasonable buffer size
        // This size should be large enough to handle bursts but small enough to provide backpressure
        let channel_buffer_size =
            (config.buffer_size_ms as usize * output_sample_rate as usize) / 1000 / 10; // 1/10th of audio buffer
        let (tx, rx) = std::sync::mpsc::sync_channel::<AudioCommand>(channel_buffer_size);

        log::debug!(
            "AudioSink: Created audio processing channel with buffer size: {}",
            channel_buffer_size
        );

        let samples_queue = Arc::new(Mutex::new(Vec::new()));
        let samples_queue_clone = Arc::clone(&samples_queue);

        // Create stats with correct sample rate
        let stats = Arc::new(CpalStats::new(
            (config.buffer_size_ms as usize * output_sample_rate as usize) / 1000,
            output_sample_rate,
            &config,
        ));
        stats_clone = Arc::clone(&stats);

        let audio_state = AudioState {
            samples_queue: samples_queue_clone,
            stats: Arc::clone(&stats),
        };

        // Create and start the stream in a separate thread to avoid Send/Sync issues
        let (stream_ready_tx, stream_ready_rx) =
            std::sync::mpsc::channel::<Result<(), AudioError>>();
        let stream_thread = thread::spawn(move || -> Result<(), AudioError> {
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

                    // Copy data from our queue to the output buffer efficiently
                    let samples_to_copy = data.len().min(queue.len());

                    // Copy the samples directly from the front of the queue
                    for i in 0..samples_to_copy {
                        data[i] = queue[i];
                    }

                    // Remove the copied samples from the front of the queue
                    queue.drain(0..samples_to_copy);

                    // Fill remaining output buffer with zeros if needed
                    for i in samples_to_copy..data.len() {
                        data[i] = 0.0;
                    }

                    // Update stats
                    audio_state.stats.update_buffer_size(queue.len());

                    // Track timing
                    let elapsed = start.elapsed();
                    audio_state
                        .stats
                        .write_interval_ms
                        .store(elapsed.as_millis() as usize, Ordering::Release);
                },
                move |err| {
                    log::error!("Audio stream error: {}", err);
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
                            // Use a more efficient polling approach
                            while !is_stopped_for_stream.load(Ordering::Relaxed) {
                                std::thread::sleep(std::time::Duration::from_millis(10));
                            }
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
        let stats_for_thread = Arc::clone(&stats);
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
                            {
                                let queue = samples_queue.lock().unwrap();
                                queue.len()
                            }
                        );
                        stats_for_thread.update_buffer_size({
                            let queue = samples_queue.lock().unwrap();
                            queue.len()
                        });
                    }
                    AudioCommand::EndOfStream => {
                        log::debug!(
                            "AudioSink: Received EndOfStream signal - no more audio will be added"
                        );
                        // Signal that no more audio will be added
                        stats_for_thread
                            .end_of_stream_signaled
                            .store(true, Ordering::Release);
                        // Don't clear the queue - let remaining audio finish playing
                        // The completion detection will handle waiting for the queue to drain
                    }
                    AudioCommand::Stop => {
                        log::debug!("AudioSink: Received Stop command, clearing queue");
                        {
                            let mut samples_queue = samples_queue.lock().unwrap();
                            samples_queue.clear();
                        }
                        stats_for_thread.update_buffer_size(0);
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
            is_stopped: is_stopped_for_struct,
            audio_thread: Some(audio_thread),
            selected_format: AudioFormat {
                sample_rate: output_sample_rate,
                channels: output_channels as u32,
                sample_format: 4, // F32
            },
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

        // Don't block the async runtime - just drop the thread handle
        // The audio thread will see the stop flag and exit on its own
        if let Some(thread) = self.audio_thread.take() {
            // Drop the thread handle - the thread will be cleaned up when the process exits
            log::debug!("AudioSink: Audio thread handle dropped");
        }
    }
}

#[async_trait::async_trait]
impl AudioSink for CpalSink {
    async fn write(&self, audio_data: &[u8]) -> Result<(), AudioError> {
        if self.is_stopped.load(Ordering::Acquire) {
            return Err(AudioError::WriteError("Audio sink is stopped".to_string()));
        }

        // Convert bytes to f32 samples based on expected format
        let expected_format = self.get_format();
        let samples_per_byte = match expected_format.sample_format {
            1 => 2, // I16: 2 bytes per sample
            2 => 3, // I24: 3 bytes per sample
            3 => 4, // I32: 4 bytes per sample
            4 => 4, // F32: 4 bytes per sample
            5 => 8, // F64: 8 bytes per sample
            _ => {
                return Err(AudioError::WriteError(
                    "Unsupported sample format".to_string(),
                ))
            }
        };

        if audio_data.len() % samples_per_byte != 0 {
            return Err(AudioError::WriteError(
                "Audio data length not aligned to sample size".to_string(),
            ));
        }

        let num_samples = audio_data.len() / samples_per_byte;
        log::debug!(
            "AudioSink: Received {} bytes, expected format: {}Hz {}ch {}",
            audio_data.len(),
            expected_format.sample_rate,
            expected_format.channels,
            match expected_format.sample_format {
                1 => "I16",
                2 => "I24",
                3 => "I32",
                4 => "F32",
                5 => "F64",
                _ => "Unknown",
            }
        );

        // Convert to f32 samples (assuming F32 format for now)
        let samples: Vec<f32> = if expected_format.sample_format == 4 {
            // F32 format
            audio_data
                .chunks_exact(4)
                .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                .collect()
        } else {
            return Err(AudioError::WriteError(
                "Only F32 sample format currently supported".to_string(),
            ));
        };

        log::debug!(
            "AudioSink: Converted {} bytes to {} F32 samples",
            audio_data.len(),
            samples.len()
        );

        // Check if we should grow the buffer before sending
        if self.stats.should_apply_backpressure() {
            log::debug!(
                "AudioSink: Buffer at {}% - attempting to grow buffer",
                self.stats.buffer_percentage()
            );

            // Try to grow the buffer to accommodate more data
            if !self.stats.try_grow_buffer() {
                log::warn!(
                    "AudioSink: Buffer at maximum size ({}%), but continuing - transport-level backpressure will handle this",
                    self.stats.buffer_percentage()
                );
            }
        }

        // Send audio to the processing thread
        // For bounded channels, we use try_send() which will return Err(TrySendError::Full) if the channel is full
        // This provides natural backpressure without blocking the async runtime
        let buffer_percentage = self.stats.buffer_percentage();

        match self.audio_sender.try_send(AudioCommand::PlayAudio(samples)) {
            Ok(_) => {
                log::debug!(
                    "AudioSink: Sent {} samples to audio thread (buffer: {}%)",
                    num_samples,
                    buffer_percentage
                );
                Ok(())
            }
            Err(std::sync::mpsc::TrySendError::Full(_)) => {
                // Channel is full - this indicates backpressure
                log::warn!("AudioSink: Audio processing channel full - applying backpressure");
                Err(AudioError::BufferFull)
            }
            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => Err(AudioError::WriteError(
                "Audio processing thread has stopped".to_string(),
            )),
        }
    }

    async fn signal_end_of_stream(&self) -> Result<(), AudioError> {
        self.audio_sender
            .try_send(AudioCommand::EndOfStream)
            .map_err(|_| AudioError::WriteError("Failed to signal end of stream".to_string()))?;
        Ok(())
    }

    async fn wait_for_completion(&self) -> Result<(), AudioError> {
        // Wait until the buffer is empty AND end of stream has been signaled
        let mut attempts = 0;
        const MAX_ATTEMPTS: u32 = 1000; // 10 seconds with 10ms intervals

        log::info!(
            "AudioSink: Starting wait for completion, max buffer: {}",
            self.stats.get_max_buffer_samples()
        );

        while attempts < MAX_ATTEMPTS {
            let buffer_samples = self.stats.buffer_samples.load(Ordering::Acquire);
            let end_of_stream_signaled = self.stats.end_of_stream_signaled.load(Ordering::Acquire);

            log::debug!(
                "AudioSink: Buffer state - samples: {}, end_of_stream: {}, attempt: {}",
                buffer_samples,
                end_of_stream_signaled,
                attempts
            );

            // Only complete when both conditions are met:
            // 1. End of stream has been signaled (no more data coming)
            // 2. Buffer is empty (all data has been played)
            if end_of_stream_signaled && buffer_samples == 0 {
                log::info!("AudioSink: All audio has been played successfully");
                return Ok(());
            }

            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
            attempts += 1;
        }

        log::warn!("AudioSink: Timeout waiting for audio completion");
        Err(AudioError::WriteError(
            "Timeout waiting for audio completion".to_string(),
        ))
    }

    async fn stop(&self) -> Result<(), AudioError> {
        self.is_stopped.store(true, Ordering::Release);
        self.audio_sender
            .try_send(AudioCommand::Stop)
            .map_err(|_| AudioError::StopError("Failed to send stop command".to_string()))?;
        Ok(())
    }

    fn get_format(&self) -> AudioFormat {
        self.selected_format
    }

    fn is_backpressure_active(&self) -> bool {
        self.stats.should_apply_backpressure()
    }

    fn get_buffer_percentage(&self) -> u8 {
        self.stats.buffer_percentage()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[cfg(feature = "audio_available")]
    async fn test_cpal_sink_creation() -> Result<(), AudioError> {
        let config = CpalConfig::default();
        let sink = CpalSink::new(config)?;
        // Explicitly stop the sink to ensure proper cleanup
        sink.stop().await?;
        Ok(())
    }

    #[tokio::test]
    #[cfg(feature = "audio_available")]
    async fn test_cpal_sink_write() -> Result<(), AudioError> {
        let config = CpalConfig::default();
        let sink = CpalSink::new(config)?;

        // Generate 1 second of silence
        let mut audio_data = Vec::new();
        for _ in 0..16000 {
            audio_data.extend_from_slice(&[0u8, 0u8]); // 16-bit PCM silence
        }

        sink.write(&audio_data).await?;
        // Explicitly stop the sink to ensure proper cleanup
        sink.stop().await?;
        Ok(())
    }

    #[tokio::test]
    #[cfg(feature = "audio_available")]
    async fn test_cpal_sink_stop() -> Result<(), AudioError> {
        let config = CpalConfig::default();
        let sink = CpalSink::new(config)?;
        sink.stop().await?;
        Ok(())
    }

    #[tokio::test]
    #[cfg(not(feature = "audio_available"))]
    async fn test_cpal_sink_creation_skipped() {
        // This test is skipped when audio hardware is not available
        assert!(true);
    }

    #[tokio::test]
    #[cfg(not(feature = "audio_available"))]
    async fn test_cpal_sink_write_skipped() {
        // This test is skipped when audio hardware is not available
        assert!(true);
    }

    #[tokio::test]
    #[cfg(not(feature = "audio_available"))]
    async fn test_cpal_sink_stop_skipped() {
        // This test is skipped when audio hardware is not available
        assert!(true);
    }
}
