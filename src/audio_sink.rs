use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, SupportedStreamConfigsError};
use log::error;
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
    /// Buffer size in milliseconds (default 2000ms)
    pub buffer_size_ms: u32,
    /// Warning threshold for low buffer (percentage)
    pub low_buffer_warning: u8,
    /// Warning threshold for high buffer_warning (percentage)
    pub high_buffer_warning: u8,
}

impl Default for CpalConfig {
    fn default() -> Self {
        Self {
            buffer_size_ms: 2000,
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
        println!("AudioSink: Creating new CpalSink");
        let (audio_sender, audio_receiver) = channel();
        let stats = Arc::new(CpalStats::new(
            (config.buffer_size_ms as usize * 16000) / 1000,
        ));
        let stats_clone = Arc::clone(&stats);
        let is_stopped = Arc::new(AtomicBool::new(false));

        let host = cpal::default_host();
        println!("AudioSink: Using audio host: {:?}", host.id());

        let device = match host.default_output_device() {
            Some(dev) => {
                println!("AudioSink: Using output device: {:?}", dev.name());
                dev
            }
            None => {
                println!("AudioSink: No output device found!");
                return Err(AudioError::DeviceError(
                    "No output device found".to_string(),
                ));
            }
        };

        // Get the default output config - we'll convert our samples to match this
        let supported_config = device
            .default_output_config()
            .map_err(|e| AudioError::DeviceError(e.to_string()))?;

        println!("AudioSink: Using output config: {:?}", supported_config);

        let output_sample_rate = supported_config.sample_rate().0;
        let output_channels = supported_config.channels() as usize;

        // Our input is always mono 16kHz
        let input_sample_rate = 16000;

        let samples_queue = Arc::new(Mutex::new(Vec::new()));
        let samples_queue_clone = Arc::clone(&samples_queue);

        let audio_thread = thread::spawn(move || {
            println!("AudioSink: Audio thread started");
            let stream = match device.build_output_stream(
                &supported_config.config(),
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    let mut queue = samples_queue_clone.lock().unwrap();
                    let initial_len = queue.len();

                    // Calculate how many input samples we need for this output buffer
                    let output_frames = data.len() / output_channels;
                    let input_samples_needed = (output_frames as f32 * input_sample_rate as f32
                        / output_sample_rate as f32)
                        .ceil() as usize;

                    // Fill output buffer with available samples or silence
                    let mut input_sample_idx: f32 = 0.0;
                    let input_sample_step = input_sample_rate as f32 / output_sample_rate as f32;

                    for frame in data.chunks_mut(output_channels) {
                        // Get the input sample using linear interpolation
                        let sample = if !queue.is_empty() {
                            let idx_floor = input_sample_idx.floor() as usize;
                            let idx_ceil = (input_sample_idx + 1.0).floor() as usize;
                            let fract = input_sample_idx.fract();

                            let sample1 = if idx_floor < queue.len() {
                                queue[idx_floor]
                            } else {
                                0.0
                            };

                            let sample2 = if idx_ceil < queue.len() {
                                queue[idx_ceil]
                            } else {
                                0.0
                            };

                            sample1 * (1.0 - fract) + sample2 * fract
                        } else {
                            0.0
                        };

                        // Write the sample to all channels
                        for channel in frame.iter_mut() {
                            *channel = sample;
                        }

                        input_sample_idx += input_sample_step;
                    }

                    // Remove used samples
                    if input_samples_needed <= queue.len() {
                        queue.drain(0..input_samples_needed);
                    } else {
                        queue.clear();
                    }

                    let samples_played = initial_len - queue.len();
                    if samples_played > 0 {
                        println!(
                            "AudioSink: Played {} samples ({} remaining)",
                            samples_played,
                            queue.len()
                        );
                    }

                    stats_clone.update_buffer_size(queue.len());
                },
                move |err| {
                    println!("AudioSink: Stream error: {}", err);
                },
                None,
            ) {
                Ok(stream) => stream,
                Err(e) => {
                    println!("AudioSink: Failed to create audio stream: {}", e);
                    return;
                }
            };

            println!("AudioSink: Starting audio playback stream");
            if let Err(e) = stream.play() {
                println!("AudioSink: Failed to start audio stream: {}", e);
                return;
            }

            println!("AudioSink: Audio stream started successfully");

            while let Ok(command) = audio_receiver.recv() {
                match command {
                    AudioCommand::PlayAudio(audio_data) => {
                        println!(
                            "AudioSink: Received {} bytes of audio data",
                            audio_data.len()
                        );
                        let mut queue = samples_queue.lock().unwrap();

                        // Convert i16 samples to f32
                        for chunk in audio_data.chunks_exact(2) {
                            let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
                            queue.push(sample as f32 / i16::MAX as f32);
                        }
                        println!("AudioSink: Converted and queued {} samples", queue.len());
                    }
                    AudioCommand::Stop => {
                        println!("AudioSink: Received stop command");
                        break;
                    }
                }
            }

            println!("AudioSink: Audio thread exiting");
            // Stream is automatically dropped here when thread exits
        });

        println!("AudioSink: Successfully created CpalSink");
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
            println!("AudioSink: Cannot write - sink is stopped");
            return Err(AudioError::WriteError("Sink is stopped".to_string()));
        }

        let buffer_percentage = self.stats.buffer_percentage();
        if buffer_percentage > self.config.high_buffer_warning {
            println!(
                "AudioSink: Buffer high warning: {}% (threshold: {}%)",
                buffer_percentage, self.config.high_buffer_warning
            );
        } else if buffer_percentage < self.config.low_buffer_warning {
            println!(
                "AudioSink: Buffer low warning: {}% (threshold: {}%)",
                buffer_percentage, self.config.low_buffer_warning
            );
        }

        if buffer_percentage >= 100 {
            println!("AudioSink: Buffer full!");
            return Err(AudioError::BufferFull);
        }

        println!(
            "AudioSink: Writing {} bytes of audio data (buffer: {}%)",
            audio_data.len(),
            buffer_percentage
        );

        match self
            .audio_sender
            .send(AudioCommand::PlayAudio(audio_data.to_vec()))
        {
            Ok(_) => println!("AudioSink: Successfully queued audio data"),
            Err(e) => {
                println!("AudioSink: Failed to queue audio data: {}", e);
                return Err(AudioError::WriteError(e.to_string()));
            }
        }

        let mut last_write = self.stats.last_write.lock().unwrap();
        let now = Instant::now();
        let interval = now.duration_since(*last_write).as_millis() as usize;
        self.stats
            .write_interval_ms
            .store(interval, Ordering::Release);
        *last_write = now;

        println!("AudioSink: Write complete (interval: {}ms)", interval);

        Ok(())
    }

    async fn stop(&self) -> Result<(), AudioError> {
        println!("AudioSink: Stopping sink");
        self.is_stopped.store(true, Ordering::Release);
        match self.audio_sender.send(AudioCommand::Stop) {
            Ok(_) => println!("AudioSink: Successfully sent stop command"),
            Err(e) => {
                println!("AudioSink: Failed to send stop command: {}", e);
                return Err(AudioError::StopError(e.to_string()));
            }
        }
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
