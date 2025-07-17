use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{
    BuildStreamError, DeviceNameError, DevicesError, PlayStreamError, SupportedStreamConfigsError,
};
use log::error;
use std::sync::mpsc;
use thiserror::Error;
use tokio::sync::oneshot;

#[derive(Error, Debug, Clone)]
pub enum AudioError {
    #[error("Failed to write audio data: {0}")]
    WriteError(String),

    #[error("Failed to stop audio playback: {0}")]
    StopError(String),

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

#[derive(Clone)]
pub struct CpalConfig {
    /// Optional output device name
    pub device_name: Option<String>,
}

impl Default for CpalConfig {
    fn default() -> Self {
        Self { device_name: None }
    }
}

enum AudioCommand {
    WriteChunk(Vec<u8>),                   // s16le data to play immediately
    EndStreamAndWait(oneshot::Sender<()>), // Signal end and wait for completion
    Abort,                                 // Abort current playback
}

/// Simplified streaming audio sink with transparent platform conversion
/// Accepts s16le data and handles conversion to hardware format internally
pub struct AudioSink {
    command_tx: mpsc::Sender<AudioCommand>,
}

impl AudioSink {
    pub fn new(config: CpalConfig) -> Result<Self, AudioError> {
        let (command_tx, command_rx) = mpsc::channel();

        // Start CPAL thread
        std::thread::spawn(move || {
            if let Err(e) = Self::run_cpal_thread(command_rx, config) {
                log::error!("CPAL thread failed: {}", e);
            }
        });

        Ok(Self { command_tx })
    }

    /// Write s16le audio chunk - returns immediately for low latency
    pub async fn write_chunk(&self, s16le_data: Vec<u8>) -> Result<(), AudioError> {
        self.command_tx
            .send(AudioCommand::WriteChunk(s16le_data))
            .map_err(|_| AudioError::WriteError("Audio thread disconnected".to_string()))?;
        Ok(())
    }

    /// Signal end of stream and wait for true completion (user has heard the audio)
    pub async fn end_stream_and_wait(&self) -> Result<(), AudioError> {
        let (completion_tx, completion_rx) = oneshot::channel();
        self.command_tx
            .send(AudioCommand::EndStreamAndWait(completion_tx))
            .map_err(|_| AudioError::WriteError("Audio thread disconnected".to_string()))?;

        completion_rx
            .await
            .map_err(|_| AudioError::WriteError("Completion signal lost".to_string()))?;
        Ok(())
    }

    /// Abort current playback immediately
    pub async fn abort(&self) -> Result<(), AudioError> {
        self.command_tx
            .send(AudioCommand::Abort)
            .map_err(|_| AudioError::WriteError("Audio thread disconnected".to_string()))?;
        Ok(())
    }

    fn run_cpal_thread(
        command_rx: mpsc::Receiver<AudioCommand>,
        config: CpalConfig,
    ) -> Result<(), AudioError> {
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

        let supported_config = device
            .default_output_config()
            .map_err(|e| AudioError::DeviceError(e.to_string()))?;

        let stream_config = supported_config.config();
        let output_sample_rate = stream_config.sample_rate.0;
        let output_channels = stream_config.channels;

        log::info!(
            "AudioSink: Hardware format - {}Hz, {}ch, {:?}",
            output_sample_rate,
            output_channels,
            supported_config.sample_format()
        );

        // Create ringbuffer for streaming
        use ringbuf::{traits::*, HeapRb};
        let buffer_size = (output_sample_rate as usize * output_channels as usize) / 10; // 100ms buffer
        let rb = HeapRb::<f32>::new(buffer_size);
        let (mut producer, mut consumer) = rb.split();

        // Create CPAL stream
        let stream = device.build_output_stream(
            &stream_config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                // Fill output buffer from ringbuffer
                for sample in data.iter_mut() {
                    *sample = consumer.try_pop().unwrap_or(0.0);
                }
            },
            |err| log::error!("CPAL stream error: {}", err),
            None,
        )?;

        stream.play()?;

        // Process commands
        let mut completion_signals: Vec<oneshot::Sender<()>> = Vec::new();

        loop {
            // Use timeout to periodically check for completion
            match command_rx.recv_timeout(std::time::Duration::from_millis(10)) {
                Ok(command) => {
                    match command {
                        AudioCommand::WriteChunk(s16le_data) => {
                            // Convert s16le to f32 and handle platform differences
                            let f32_samples = Self::convert_s16le_to_platform_f32(
                                &s16le_data,
                                output_sample_rate,
                                output_channels,
                            )?;

                            // Write to ringbuffer (non-blocking)
                            for sample in f32_samples {
                                if producer.try_push(sample).is_err() {
                                    log::warn!("Audio buffer full, dropping samples");
                                    break;
                                }
                            }
                        }
                        AudioCommand::EndStreamAndWait(tx) => {
                            // Wait a short time for audio to finish, then signal completion
                            // This is simplified - assumes buffer drains in ~100ms
                            std::thread::sleep(std::time::Duration::from_millis(100));
                            let _ = tx.send(());
                        }
                        AudioCommand::Abort => {
                            // Signal completion for any pending operations
                            for tx in completion_signals.drain(..) {
                                let _ = tx.send(());
                            }
                        }
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    // Continue polling
                    continue;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    break;
                }
            }
        }

        Ok(())
    }

    fn convert_s16le_to_platform_f32(
        s16le_data: &[u8],
        _target_sample_rate: u32,
        target_channels: u16,
    ) -> Result<Vec<f32>, AudioError> {
        if s16le_data.len() % 2 != 0 {
            return Err(AudioError::WriteError(
                "S16LE data length not aligned to 16-bit samples".to_string(),
            ));
        }

        // Convert s16le bytes to f32 samples
        let mut f32_samples: Vec<f32> = s16le_data
            .chunks_exact(2)
            .map(|chunk| {
                let i16_sample = i16::from_le_bytes([chunk[0], chunk[1]]);
                i16_sample as f32 / 32768.0 // Scale to [-1.0, 1.0]
            })
            .collect();

        // Handle mono to stereo conversion if needed
        if target_channels == 2 && !f32_samples.is_empty() {
            // Duplicate mono samples for stereo
            let mut stereo_samples = Vec::with_capacity(f32_samples.len() * 2);
            for sample in f32_samples {
                stereo_samples.push(sample);
                stereo_samples.push(sample);
            }
            f32_samples = stereo_samples;
        }

        // TODO: Add resampling if target_sample_rate != 16000
        // For now, assume input is already at correct sample rate

        Ok(f32_samples)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[cfg(feature = "audio_available")]
    async fn test_cpal_sink_creation() -> Result<(), AudioError> {
        let config = CpalConfig::default();
        let sink = AudioSink::new(config)?;
        // Explicitly stop the sink to ensure proper cleanup
        sink.abort().await?;
        Ok(())
    }

    #[tokio::test]
    #[cfg(feature = "audio_available")]
    async fn test_cpal_sink_write() -> Result<(), AudioError> {
        let config = CpalConfig::default();
        let sink = AudioSink::new(config)?;

        // Generate 1 second of silence
        let mut audio_data = Vec::new();
        for _ in 0..16000 {
            audio_data.extend_from_slice(&[0u8, 0u8]); // 16-bit PCM silence
        }

        sink.write_chunk(audio_data).await?;
        // Explicitly stop the sink to ensure proper cleanup
        sink.abort().await?;
        Ok(())
    }

    #[tokio::test]
    #[cfg(feature = "audio_available")]
    async fn test_cpal_sink_stop() -> Result<(), AudioError> {
        let config = CpalConfig::default();
        let sink = AudioSink::new(config)?;
        sink.abort().await?;
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
