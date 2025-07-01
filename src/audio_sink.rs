use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, SupportedStreamConfigsError};
use log::{debug, error, warn};
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
    /// Buffer size in milliseconds (default 30000ms = 30s)
    pub buffer_size_ms: u32,
    /// Warning threshold for low buffer (percentage)
    pub low_buffer_warning: u8,
    /// Warning threshold for high buffer_warning (percentage)
    pub high_buffer_warning: u8,
}

impl Default for CpalConfig {
    fn default() -> Self {
        Self {
            buffer_size_ms: 30000,
            low_buffer_warning: 20,
            high_buffer_warning: 80,
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
    PlayAudio(Vec<u8>),
    Stop,
}

pub struct CpalSink {
    audio_sender: Sender<AudioCommand>,
    stats: Arc<CpalStats>,
    config: CpalConfig,
    is_stopped: Arc<AtomicBool>,
    audio_thread: Option<thread::JoinHandle<()>>,
}

impl CpalSink {
    pub fn new(config: CpalConfig) -> Result<Self, AudioError> {
        let (audio_sender, audio_receiver) = channel();
        let stats = Arc::new(CpalStats::new(
            (config.buffer_size_ms as usize * 16000) / 1000,
        ));
        let stats_clone = Arc::clone(&stats);
        let is_stopped = Arc::new(AtomicBool::new(false));

        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| AudioError::DeviceError("No output device found".to_string()))?;

        let supported_config = device
            .supported_output_configs()?
            .find(|config| {
                config.channels() == 1
                    && config.sample_format() == SampleFormat::F32
                    && config.min_sample_rate() <= cpal::SampleRate(16000)
                    && config.max_sample_rate() >= cpal::SampleRate(16000)
            })
            .ok_or_else(|| AudioError::DeviceError("No suitable output config found".to_string()))?
            .with_sample_rate(cpal::SampleRate(16000));

        let samples_queue = Arc::new(Mutex::new(Vec::new()));
        let samples_queue_clone = Arc::clone(&samples_queue);

        let audio_thread = thread::spawn(move || {
            // Create the stream inside the thread to avoid Send + Sync issues
            let stream = match device.build_output_stream(
                &supported_config.into(),
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    let mut queue = samples_queue_clone.lock().unwrap();

                    // Fill output buffer with available samples or silence
                    for sample_out in data.iter_mut() {
                        if let Some(sample) = queue.pop() {
                            *sample_out = sample;
                        } else {
                            *sample_out = 0.0;
                        }
                    }

                    stats_clone.update_buffer_size(queue.len());
                },
                move |err| {
                    error!("Audio stream error: {}", err);
                },
                None,
            ) {
                Ok(stream) => stream,
                Err(e) => {
                    error!("Failed to create audio stream: {}", e);
                    return;
                }
            };

            if let Err(e) = stream.play() {
                error!("Failed to start audio stream: {}", e);
                return;
            }

            while let Ok(command) = audio_receiver.recv() {
                match command {
                    AudioCommand::PlayAudio(audio_data) => {
                        let mut queue = samples_queue.lock().unwrap();

                        // Convert i16 samples to f32
                        for chunk in audio_data.chunks_exact(2) {
                            let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
                            queue.push(sample as f32 / i16::MAX as f32);
                        }
                    }
                    AudioCommand::Stop => {
                        break;
                    }
                }
            }

            // Stream is automatically dropped here when thread exits
        });

        Ok(Self {
            audio_sender,
            stats,
            config,
            is_stopped,
            audio_thread: Some(audio_thread),
        })
    }

    pub fn get_stats(&self) -> (u8, usize) {
        (
            self.stats.buffer_percentage(),
            self.stats.write_interval_ms.load(Ordering::Acquire),
        )
    }
}

impl Drop for CpalSink {
    fn drop(&mut self) {
        if !self.is_stopped.load(Ordering::Acquire) {
            if let Err(e) = self.audio_sender.send(AudioCommand::Stop) {
                error!("Failed to send stop command: {}", e);
            }
        }

        if let Some(thread) = self.audio_thread.take() {
            if let Err(e) = thread.join() {
                error!("Failed to join audio thread: {:?}", e);
            }
        }
    }
}

#[async_trait::async_trait]
impl AudioSink for CpalSink {
    async fn write(&self, audio_data: &[u8]) -> Result<(), AudioError> {
        if self.is_stopped.load(Ordering::Acquire) {
            return Err(AudioError::WriteError("Sink is stopped".to_string()));
        }

        let buffer_percentage = self.stats.buffer_percentage();
        if buffer_percentage > self.config.high_buffer_warning {
            warn!(
                "Audio buffer is high: {}% (threshold: {}%)",
                buffer_percentage, self.config.high_buffer_warning
            );
        } else if buffer_percentage < self.config.low_buffer_warning {
            debug!(
                "Audio buffer is low: {}% (threshold: {}%)",
                buffer_percentage, self.config.low_buffer_warning
            );
        }

        if buffer_percentage >= 100 {
            return Err(AudioError::BufferFull);
        }

        self.audio_sender
            .send(AudioCommand::PlayAudio(audio_data.to_vec()))
            .map_err(|e| AudioError::WriteError(e.to_string()))?;

        let mut last_write = self.stats.last_write.lock().unwrap();
        let now = Instant::now();
        let interval = now.duration_since(*last_write).as_millis() as usize;
        self.stats
            .write_interval_ms
            .store(interval, Ordering::Release);
        *last_write = now;

        Ok(())
    }

    async fn stop(&self) -> Result<(), AudioError> {
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

    #[tokio::test]
    async fn test_cpal_sink_creation() -> Result<(), AudioError> {
        let config = CpalConfig::default();
        match CpalSink::new(config) {
            Ok(sink) => {
                assert!(!sink.is_stopped.load(Ordering::Acquire));
                Ok(())
            }
            Err(e) => {
                println!(
                    "Audio device not available in test environment - this is expected: {}",
                    e
                );
                Ok(())
            }
        }
    }

    #[tokio::test]
    async fn test_cpal_sink_write() -> Result<(), AudioError> {
        let config = CpalConfig::default();
        match CpalSink::new(config) {
            Ok(sink) => {
                // Generate 1 second of 440Hz sine wave
                let sample_rate = 16000;
                let duration = 1.0;
                let frequency = 440.0;
                let num_samples = (sample_rate as f32 * duration) as usize;
                let mut samples = Vec::with_capacity(num_samples * 2);

                for i in 0..num_samples {
                    let t = i as f32 / sample_rate as f32;
                    let value = (2.0 * std::f32::consts::PI * frequency * t).sin();
                    let sample = (value * i16::MAX as f32) as i16;
                    samples.extend_from_slice(&sample.to_le_bytes());
                }

                sink.write(&samples).await?;
                Ok(())
            }
            Err(e) => {
                println!(
                    "Audio device not available in test environment - this is expected: {}",
                    e
                );
                Ok(())
            }
        }
    }

    #[tokio::test]
    async fn test_cpal_sink_stop() -> Result<(), AudioError> {
        let config = CpalConfig::default();
        match CpalSink::new(config) {
            Ok(sink) => {
                sink.stop().await?;
                assert!(sink.is_stopped.load(Ordering::Acquire));
                Ok(())
            }
            Err(e) => {
                println!(
                    "Audio device not available in test environment - this is expected: {}",
                    e
                );
                Ok(())
            }
        }
    }
}
