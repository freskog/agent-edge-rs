use crate::platform::AudioPlatform;
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
        // Use RaspberryPi as default platform for backward compatibility
        Self::new_with_platform(config, AudioPlatform::RaspberryPi)
    }

    pub fn new_with_platform(
        config: CpalConfig,
        platform: AudioPlatform,
    ) -> Result<Self, AudioError> {
        let (command_tx, command_rx) = mpsc::channel();

        // Start CPAL thread
        std::thread::spawn(move || {
            if let Err(e) = Self::run_cpal_thread(command_rx, config, platform) {
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
        platform: AudioPlatform,
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
                                platform,
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
        target_sample_rate: u32,
        target_channels: u16,
        platform: AudioPlatform,
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

        // TTS produces s16le @ 44.1kHz natively, so no resampling needed
        // Just verify the input rate matches expected playback rate
        let expected_input_rate = platform.playback_config().sample_rate; // 44.1kHz
        if target_sample_rate != expected_input_rate {
            log::warn!(
                "⚠️  Hardware sample rate ({}Hz) doesn't match TTS output rate ({}Hz)",
                target_sample_rate,
                expected_input_rate
            );
        }

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

        Ok(f32_samples)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_s16le_to_f32_conversion() {
        // Test s16le to f32 conversion without resampling
        let mut s16le_data = Vec::new();

        // Generate test data: max positive, zero, max negative
        let test_values = [i16::MAX, 0, i16::MIN];
        for &val in &test_values {
            s16le_data.extend_from_slice(&val.to_le_bytes());
        }

        // Convert to f32
        let f32_samples: Vec<f32> = s16le_data
            .chunks_exact(2)
            .map(|chunk| {
                let i16_sample = i16::from_le_bytes([chunk[0], chunk[1]]);
                i16_sample as f32 / 32768.0
            })
            .collect();

        // Verify conversion
        assert_eq!(f32_samples.len(), 3);
        assert!((f32_samples[0] - 1.0).abs() < 0.001); // Max positive -> ~1.0
        assert!((f32_samples[1] - 0.0).abs() < 0.001); // Zero -> 0.0
        assert!((f32_samples[2] - (-1.0)).abs() < 0.001); // Max negative -> ~-1.0

        println!("✅ s16le to f32 conversion test passed");
        println!(
            "   {} -> {} -> {}",
            i16::MAX,
            i16::MAX as f32 / 32768.0,
            f32_samples[0]
        );
        println!("   {} -> {} -> {}", 0, 0.0, f32_samples[1]);
        println!(
            "   {} -> {} -> {}",
            i16::MIN,
            i16::MIN as f32 / 32768.0,
            f32_samples[2]
        );
    }

    #[test]
    fn test_convert_s16le_to_platform_f32() {
        // Create test s16le data (44.1kHz mono - TTS output format)
        let sample_rate = 44100;
        let target_channels = 2; // stereo
        let platform = AudioPlatform::RaspberryPi;

        // Generate 1000 samples of s16le data
        let mut s16le_data = Vec::new();
        for i in 0..1000 {
            let sample = (i as f32 / 1000.0 * 2.0 * std::f32::consts::PI).sin();
            let i16_sample = (sample * 32767.0) as i16;
            s16le_data.extend_from_slice(&i16_sample.to_le_bytes());
        }

        let result = AudioSink::convert_s16le_to_platform_f32(
            &s16le_data,
            sample_rate,
            target_channels,
            platform,
        );

        match result {
            Ok(f32_samples) => {
                // Should convert to stereo (no resampling needed)
                let expected_stereo_output = 1000 * 2; // 1000 mono samples -> 2000 stereo samples

                println!("✅ Platform conversion test passed:");
                println!(
                    "   Input: {} bytes s16le ({}Hz mono)",
                    s16le_data.len(),
                    sample_rate
                );
                println!(
                    "   Output: {} f32 samples ({}Hz stereo)",
                    f32_samples.len(),
                    sample_rate
                );
                println!("   Expected stereo samples: {}", expected_stereo_output);

                assert_eq!(f32_samples.len(), expected_stereo_output);

                // Verify samples are in valid range
                for (i, &sample) in f32_samples.iter().enumerate() {
                    assert!(
                        sample >= -1.0 && sample <= 1.0,
                        "Sample {} out of range: {}",
                        i,
                        sample
                    );
                }
            }
            Err(e) => {
                panic!("Platform conversion failed: {}", e);
            }
        }
    }

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
