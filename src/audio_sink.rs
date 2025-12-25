use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{
    BuildStreamError, DeviceNameError, DevicesError, PlayStreamError, SampleFormat, Stream,
    SupportedStreamConfigsError,
};
use crossbeam::channel::{bounded, Receiver, Sender};
use log::error;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;
use thiserror::Error;

// Use the existing platform infrastructure
use crate::platform::{AudioPlatform, PlatformSampleFormat};

// Build-time platform detection using existing types
#[cfg(target_os = "macos")]
const PLATFORM: AudioPlatform = AudioPlatform::MacOS;
#[cfg(target_os = "linux")]
const PLATFORM: AudioPlatform = AudioPlatform::RaspberryPi;

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
pub struct AudioSinkConfig {
    /// Optional output device name
    pub device_name: Option<String>,
}

impl Default for AudioSinkConfig {
    fn default() -> Self {
        Self { device_name: None }
    }
}

enum AudioCommand {
    WriteChunk(Vec<u8>),                // mono 48kHz s16le data to play
    EndStreamAndWait(mpsc::Sender<()>), // Signal end and wait for completion
    Abort,                              // Abort current playback
}

/// Platform-aware audio buffer that avoids unnecessary conversions
enum PlatformAudioBuffer {
    I16(Vec<i16>), // For Raspberry Pi - stays in integer domain
    F32(Vec<f32>), // For macOS - uses floating point
}

impl PlatformAudioBuffer {
    /// Create buffer based on actual stream format (fixes cross-platform format mismatch)
    fn new_with_stream_format(stream_format: SampleFormat) -> Self {
        match stream_format {
            SampleFormat::I16 => {
                log::info!("🔊 Using I16 audio buffer (matches stream format)");
                Self::I16(Vec::new())
            }
            SampleFormat::F32 => {
                log::info!("🔊 Using F32 audio buffer (matches stream format)");
                Self::F32(Vec::new())
            }
            _ => {
                log::warn!(
                    "⚠️  Unsupported stream format {:?}, defaulting to F32 buffer",
                    stream_format
                );
                Self::F32(Vec::new())
            }
        }
    }

    /// Add s16le data to buffer with platform-optimized conversion
    fn extend_from_s16le(
        &mut self,
        s16le_data: &[u8],
        target_channels: u16,
    ) -> Result<(), AudioError> {
        if s16le_data.len() % 2 != 0 {
            return Err(AudioError::WriteError(
                "S16LE data length not aligned to 16-bit samples".to_string(),
            ));
        }

        match self {
            Self::I16(buffer) => {
                // Raspberry Pi path: s16le → s16 → stereo s16 (no f32 conversion!)
                let mono_samples: Vec<i16> = s16le_data
                    .chunks_exact(2)
                    .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
                    .collect();

                if target_channels == 2 {
                    // Convert mono to stereo in integer domain
                    for sample in mono_samples {
                        buffer.push(sample); // Left channel
                        buffer.push(sample); // Right channel
                    }
                } else if target_channels == 1 {
                    buffer.extend_from_slice(&mono_samples);
                } else {
                    return Err(AudioError::WriteError(format!(
                        "Unsupported channel count: {}",
                        target_channels
                    )));
                }
            }
            Self::F32(buffer) => {
                // macOS path: s16le → f32 → stereo f32 (conversion needed for hardware)
                let mono_samples: Vec<f32> = s16le_data
                    .chunks_exact(2)
                    .map(|chunk| {
                        let i16_sample = i16::from_le_bytes([chunk[0], chunk[1]]);
                        i16_sample as f32 / 32768.0 // Scale to [-1.0, 1.0]
                    })
                    .collect();

                if target_channels == 2 {
                    // Convert mono to stereo
                    for sample in mono_samples {
                        buffer.push(sample); // Left channel
                        buffer.push(sample); // Right channel
                    }
                } else if target_channels == 1 {
                    buffer.extend_from_slice(&mono_samples);
                } else {
                    return Err(AudioError::WriteError(format!(
                        "Unsupported channel count: {}",
                        target_channels
                    )));
                }
            }
        }
        Ok(())
    }

    fn len(&self) -> usize {
        match self {
            Self::I16(buffer) => buffer.len(),
            Self::F32(buffer) => buffer.len(),
        }
    }

    fn clear(&mut self) {
        match self {
            Self::I16(buffer) => buffer.clear(),
            Self::F32(buffer) => buffer.clear(),
        }
    }

    /// Extract samples for I16 stream callback
    fn extract_i16_samples(&mut self, count: usize) -> (Vec<i16>, bool) {
        match self {
            Self::I16(buffer) => {
                let available = buffer.len();
                if available >= count {
                    let samples = buffer.drain(..count).collect();
                    (samples, false) // No underrun
                } else {
                    let samples = buffer.drain(..).collect();
                    (samples, available < count) // Underrun if we don't have enough
                }
            }
            Self::F32(_) => {
                log::error!("Attempted to extract I16 samples from F32 buffer!");
                (vec![0; count], true)
            }
        }
    }

    /// Extract samples for F32 stream callback
    fn extract_f32_samples(&mut self, count: usize) -> (Vec<f32>, bool) {
        match self {
            Self::F32(buffer) => {
                let available = buffer.len();
                if available >= count {
                    let samples = buffer.drain(..count).collect();
                    (samples, false) // No underrun
                } else {
                    let samples = buffer.drain(..).collect();
                    (samples, available < count) // Underrun if we don't have enough
                }
            }
            Self::I16(_) => {
                log::error!("Attempted to extract F32 samples from I16 buffer!");
                (vec![0.0; count], true)
            }
        }
    }
}

/// Sync streaming audio sink
/// Accepts mono 48kHz s16le and converts to hardware format (stereo)
pub struct AudioSink {
    command_tx: Sender<AudioCommand>,
    _handle: thread::JoinHandle<()>,
}

impl AudioSink {
    /// Create a new audio sink using build-time detected platform
    pub fn new(config: AudioSinkConfig) -> Result<Self, AudioError> {
        // Reduced from 100 to 20 for more responsive barge-in
        // At 48kHz with typical chunk sizes, 20 slots is still ~200-400ms of buffering
        let (command_tx, command_rx) = bounded(20);

        // Start CPAL thread
        let handle = thread::spawn(move || {
            if let Err(e) = Self::run_cpal_thread(command_rx, config) {
                log::error!("CPAL thread failed: {}", e);
            }
        });

        Ok(Self {
            command_tx,
            _handle: handle,
        })
    }

    /// Write mono 48kHz s16le audio chunk - returns immediately for low latency
    pub fn write_chunk(&self, s16le_data: Vec<u8>) -> Result<(), AudioError> {
        self.command_tx
            .send(AudioCommand::WriteChunk(s16le_data))
            .map_err(|_| AudioError::WriteError("Audio thread disconnected".to_string()))?;
        Ok(())
    }

    /// Signal end of stream and wait for completion (blocking)
    pub fn end_stream_and_wait(&self) -> Result<(), AudioError> {
        let (completion_tx, completion_rx) = mpsc::channel();
        self.command_tx
            .send(AudioCommand::EndStreamAndWait(completion_tx))
            .map_err(|_| AudioError::WriteError("Audio thread disconnected".to_string()))?;

        completion_rx
            .recv()
            .map_err(|_| AudioError::WriteError("Completion signal lost".to_string()))?;
        Ok(())
    }

    /// Signal end of stream (non-blocking) - returns a receiver to check completion
    pub fn end_stream(&self) -> Result<mpsc::Receiver<()>, AudioError> {
        let (completion_tx, completion_rx) = mpsc::channel();
        self.command_tx
            .send(AudioCommand::EndStreamAndWait(completion_tx))
            .map_err(|_| AudioError::WriteError("Audio thread disconnected".to_string()))?;
        Ok(completion_rx)
    }

    /// Abort current playback immediately
    pub fn abort(&self) -> Result<(), AudioError> {
        self.command_tx
            .send(AudioCommand::Abort)
            .map_err(|_| AudioError::WriteError("Audio thread disconnected".to_string()))?;
        Ok(())
    }

    fn run_cpal_thread(
        command_rx: Receiver<AudioCommand>,
        config: AudioSinkConfig,
    ) -> Result<(), AudioError> {
        let host = cpal::default_host();
        let platform = PLATFORM;
        let platform_config = platform.playback_config();

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

        log::info!("🔊 Using output device: {:?}", device.name());

        let supported_config = device
            .default_output_config()
            .map_err(|e| AudioError::DeviceError(e.to_string()))?;

        let stream_config = supported_config.config();
        let hardware_sample_rate = stream_config.sample_rate.0;
        let hardware_channels = stream_config.channels;
        let hardware_format = supported_config.sample_format();

        log::info!(
            "🔊 Platform: {} wants {:?}, Hardware: {}Hz, {}ch, {:?}",
            platform,
            platform_config.format,
            hardware_sample_rate,
            hardware_channels,
            hardware_format
        );

        // Choose optimal format based on platform preference and hardware support
        let use_format = match (&platform_config.format, hardware_format) {
            (PlatformSampleFormat::I16, SampleFormat::I16) => {
                log::info!("✅ Perfect match: using I16 format (no conversions)");
                SampleFormat::I16
            }
            (PlatformSampleFormat::F32, _) => {
                log::info!("✅ Using F32 format as preferred by platform");
                SampleFormat::F32
            }
            (PlatformSampleFormat::I16, _) => {
                log::warn!("⚠️  Hardware doesn't support I16, falling back to F32");
                SampleFormat::F32
            }
        };

        // Create platform-appropriate buffer
        let audio_buffer = Arc::new(Mutex::new(PlatformAudioBuffer::new_with_stream_format(
            use_format,
        )));

        // Create stream based on chosen format
        let stream = match use_format {
            SampleFormat::I16 => {
                Self::create_i16_stream(&device, &stream_config, Arc::clone(&audio_buffer))?
            }
            SampleFormat::F32 => {
                Self::create_f32_stream(&device, &stream_config, Arc::clone(&audio_buffer))?
            }
            _ => {
                return Err(AudioError::DeviceError(format!(
                    "Unsupported sample format: {:?}",
                    use_format
                )));
            }
        };

        stream.play()?;

        // Process commands
        let mut completion_signals: Vec<mpsc::Sender<()>> = Vec::new();

        loop {
            // Use timeout to periodically check for completion
            match command_rx.recv_timeout(Duration::from_millis(10)) {
                Ok(command) => {
                    match command {
                        AudioCommand::WriteChunk(s16le_data) => {
                            // Add to platform-appropriate buffer
                            // Let the bounded channel provide natural backpressure instead of dropping samples
                            {
                                let mut buffer = audio_buffer.lock().unwrap();
                                buffer.extend_from_s16le(&s16le_data, hardware_channels)?;
                            }

                            // CRITICAL FIX: Check completion signals after EVERY chunk
                            // This prevents buffering 1-2 minutes of audio before checking drain status
                            if !completion_signals.is_empty() {
                                // Check BOTH command queue AND playback buffer
                                let queue_is_empty = command_rx.is_empty();
                                let buffer_len = {
                                    let buffer = audio_buffer.lock().unwrap();
                                    buffer.len()
                                };

                                // Only signal completion if:
                                // 1. No more commands in queue (all audio sent)
                                // 2. Playback buffer is nearly empty (< 20ms remaining)
                                if queue_is_empty && buffer_len < hardware_sample_rate as usize / 50
                                {
                                    log::debug!(
                                        "✅ Playback complete: queue empty, buffer={} samples (< 20ms)",
                                        buffer_len
                                    );
                                    for tx in completion_signals.drain(..) {
                                        let _ = tx.send(());
                                    }
                                }
                            }
                        }
                        AudioCommand::EndStreamAndWait(tx) => {
                            completion_signals.push(tx);
                        }
                        AudioCommand::Abort => {
                            // PRIORITY HANDLING: Drain all pending commands when abort is received
                            // This prevents buffered WriteChunk commands from adding more audio
                            let mut drained_count = 0;
                            while let Ok(cmd) = command_rx.try_recv() {
                                match cmd {
                                    AudioCommand::WriteChunk(_) => {
                                        drained_count += 1;
                                        // Drop the chunk, don't add to buffer
                                    }
                                    AudioCommand::EndStreamAndWait(tx) => {
                                        completion_signals.push(tx);
                                    }
                                    AudioCommand::Abort => {
                                        // Another abort command, continue draining
                                    }
                                }
                            }

                            if drained_count > 0 {
                                log::info!(
                                    "🗑️  Drained {} buffered audio chunks during abort",
                                    drained_count
                                );
                            }

                            // Clear buffer and signal completion for any pending operations
                            {
                                let mut buffer = audio_buffer.lock().unwrap();
                                buffer.clear();
                            }
                            for tx in completion_signals.drain(..) {
                                let _ = tx.send(());
                            }
                        }
                    }
                }
                Err(crossbeam::channel::RecvTimeoutError::Timeout) => {
                    // Check if we need to signal completion
                    if !completion_signals.is_empty() {
                        // Check BOTH command queue AND playback buffer
                        let queue_is_empty = command_rx.is_empty();
                        let buffer_len = {
                            let buffer = audio_buffer.lock().unwrap();
                            buffer.len()
                        };

                        // Only signal completion if:
                        // 1. No more commands in queue (all audio sent)
                        // 2. Playback buffer is nearly empty (< 20ms remaining)
                        if queue_is_empty && buffer_len < hardware_sample_rate as usize / 50 {
                            log::debug!(
                                "✅ Playback complete (timeout check): queue empty, buffer={} samples (< 20ms)",
                                buffer_len
                            );
                            for tx in completion_signals.drain(..) {
                                let _ = tx.send(());
                            }
                        }
                    }
                }
                Err(crossbeam::channel::RecvTimeoutError::Disconnected) => {
                    break;
                }
            }
        }

        Ok(())
    }

    /// Create I16 CPAL stream (optimal for Raspberry Pi)
    fn create_i16_stream(
        device: &cpal::Device,
        config: &cpal::StreamConfig,
        audio_buffer: Arc<Mutex<PlatformAudioBuffer>>,
    ) -> Result<Stream, AudioError> {
        device
            .build_output_stream(
                config,
                move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                    let mut buffer = audio_buffer.lock().unwrap();
                    let (samples, underrun) = buffer.extract_i16_samples(data.len());

                    // Copy available samples
                    let copy_len = samples.len().min(data.len());
                    data[..copy_len].copy_from_slice(&samples[..copy_len]);

                    // Fill remainder with silence if underrun
                    if underrun && copy_len < data.len() {
                        for sample in data.iter_mut().skip(copy_len) {
                            *sample = 0;
                        }
                    }
                },
                |err| log::error!("CPAL I16 stream error: {}", err),
                None,
            )
            .map_err(AudioError::from)
    }

    /// Create F32 CPAL stream (for macOS)
    fn create_f32_stream(
        device: &cpal::Device,
        config: &cpal::StreamConfig,
        audio_buffer: Arc<Mutex<PlatformAudioBuffer>>,
    ) -> Result<Stream, AudioError> {
        device
            .build_output_stream(
                config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    let mut buffer = audio_buffer.lock().unwrap();
                    let (samples, underrun) = buffer.extract_f32_samples(data.len());

                    // Copy available samples
                    let copy_len = samples.len().min(data.len());
                    data[..copy_len].copy_from_slice(&samples[..copy_len]);

                    // Fill remainder with silence if underrun
                    if underrun && copy_len < data.len() {
                        for sample in data.iter_mut().skip(copy_len) {
                            *sample = 0.0;
                        }
                    }
                },
                |err| log::error!("CPAL F32 stream error: {}", err),
                None,
            )
            .map_err(AudioError::from)
    }

    /// List available audio devices
    pub fn list_devices() -> Result<Vec<AudioDeviceInfo>, AudioError> {
        let host = cpal::default_host();
        let devices = host
            .output_devices()
            .map_err(|e| AudioError::DeviceError(e.to_string()))?;

        let default_device = host.default_output_device();
        let default_name = default_device.and_then(|d| d.name().ok());

        let mut device_infos = Vec::new();
        for device in devices {
            let name = device
                .name()
                .map_err(|e| AudioError::DeviceError(e.to_string()))?;

            let config = device
                .default_output_config()
                .map_err(|e| AudioError::DeviceError(e.to_string()))?;

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

/// Audio device information
#[derive(Debug, Clone)]
pub struct AudioDeviceInfo {
    pub name: String,
    pub id: String,
    pub is_default: bool,
    pub channel_count: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_platform_buffer_optimization() {
        // Test data: 4 samples of s16le (8 bytes)
        let s16le_data = vec![
            0x00, 0x10, // Sample 1: 4096
            0x00, 0x20, // Sample 2: 8192
            0x00, 0x30, // Sample 3: 12288
            0x00, 0x40, // Sample 4: 16384
        ];

        // Test Raspberry Pi (I16) - should stay in integer domain
        let mut pi_buffer = PlatformAudioBuffer::I16(Vec::new());
        pi_buffer.extend_from_s16le(&s16le_data, 2).unwrap(); // Stereo

        if let PlatformAudioBuffer::I16(samples) = pi_buffer {
            // Should have 8 samples (4 mono → 8 stereo)
            assert_eq!(samples.len(), 8);
            // Values should be unchanged (no f32 conversion)
            assert_eq!(samples[0], 4096); // Left
            assert_eq!(samples[1], 4096); // Right (duplicated)
            assert_eq!(samples[2], 8192); // Left
            assert_eq!(samples[3], 8192); // Right (duplicated)
            println!("✅ Raspberry Pi: No unnecessary conversions! s16le → i16 → stereo i16");
        } else {
            panic!("Expected I16 buffer for Raspberry Pi");
        }

        // Test macOS (F32) - conversion to normalized range
        let mut mac_buffer = PlatformAudioBuffer::F32(Vec::new());
        mac_buffer.extend_from_s16le(&s16le_data, 2).unwrap(); // Stereo

        if let PlatformAudioBuffer::F32(samples) = mac_buffer {
            // Should have 8 samples (4 mono → 8 stereo)
            assert_eq!(samples.len(), 8);
            // Values should be normalized to [-1.0, 1.0] range
            assert_eq!(samples[0], 4096.0 / 32768.0); // ≈ 0.125
            assert_eq!(samples[1], 4096.0 / 32768.0); // Right (duplicated)
            assert_eq!(samples[2], 8192.0 / 32768.0); // ≈ 0.25
            assert_eq!(samples[3], 8192.0 / 32768.0); // Right (duplicated)
            println!("✅ macOS: Appropriate conversion! s16le → f32 → stereo f32");
        } else {
            panic!("Expected F32 buffer for macOS");
        }
    }

    #[test]
    fn test_platform_format_selection() {
        // Test that we select the right format for each platform
        let pi_config = AudioPlatform::RaspberryPi.playback_config();
        let mac_config = AudioPlatform::MacOS.playback_config();

        assert_eq!(pi_config.format, PlatformSampleFormat::I16);
        assert_eq!(mac_config.format, PlatformSampleFormat::F32);

        println!("✅ Platform detection working correctly:");
        println!("   Raspberry Pi → I16 (optimal for DAC)");
        println!("   macOS → F32 (optimal for Core Audio)");
    }

    #[test]
    fn test_buffer_underrun_handling() {
        let mut buffer = PlatformAudioBuffer::I16(vec![1, 2, 3]);

        // Extract more samples than available
        let (samples, underrun) = buffer.extract_i16_samples(5);

        assert_eq!(samples.len(), 3); // Only got what was available
        assert!(underrun); // Should signal underrun
        assert_eq!(buffer.len(), 0); // Buffer should be empty after extraction
    }
}
