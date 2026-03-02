use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{
    BuildStreamError, DeviceNameError, DevicesError, PlayStreamError, SampleFormat, Stream,
    SupportedStreamConfigsError,
};
use crossbeam::channel::{bounded, Receiver, Sender};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;
use thiserror::Error;

pub use crate::types::AudioDeviceInfo;

const TARGET_SAMPLE_RATE: u32 = 48000;
const TARGET_CHANNELS: u16 = 1;

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
    pub device_name: Option<String>,
}

impl Default for AudioSinkConfig {
    fn default() -> Self {
        Self { device_name: None }
    }
}

enum AudioCommand {
    WriteChunk { data: Vec<u8>, stream_id: u64 },
    EndStreamAndWait(mpsc::Sender<()>),
    Abort,
}

/// Parse s16le bytes into the i16 playback buffer.
fn extend_buffer_from_s16le(buffer: &mut Vec<i16>, s16le_data: &[u8]) -> Result<(), AudioError> {
    if s16le_data.len() % 2 != 0 {
        return Err(AudioError::WriteError(
            "S16LE data length not aligned to 16-bit samples".to_string(),
        ));
    }
    buffer.reserve(s16le_data.len() / 2);
    for chunk in s16le_data.chunks_exact(2) {
        buffer.push(i16::from_le_bytes([chunk[0], chunk[1]]));
    }
    Ok(())
}

/// Drain up to `count` samples from the buffer.
/// Returns (samples, underrun).
fn drain_samples(buffer: &mut Vec<i16>, count: usize) -> (Vec<i16>, bool) {
    let available = buffer.len();
    if available >= count {
        let samples = buffer.drain(..count).collect();
        (samples, false)
    } else {
        let samples = buffer.drain(..).collect();
        (samples, available < count)
    }
}

/// Sync streaming audio sink.
/// Accepts mono 48kHz s16le and feeds directly to I16 ALSA hardware.
pub struct AudioSink {
    command_tx: Sender<AudioCommand>,
    _handle: thread::JoinHandle<()>,
}

impl AudioSink {
    /// Blocks until the ALSA output device is fully configured so that
    /// other devices on the same card can be opened safely afterward.
    pub fn new(config: AudioSinkConfig) -> Result<Self, AudioError> {
        let (command_tx, command_rx) = bounded(20);
        let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<(), AudioError>>(1);

        let handle = thread::spawn(move || {
            if let Err(e) = Self::run_cpal_thread(command_rx, config, ready_tx) {
                log::error!("CPAL thread failed: {}", e);
            }
        });

        match ready_rx.recv() {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(e),
            Err(_) => {
                return Err(AudioError::DeviceError(
                    "Audio thread died during initialization".to_string(),
                ))
            }
        }

        Ok(Self {
            command_tx,
            _handle: handle,
        })
    }

    /// Write mono 48kHz s16le audio chunk -- returns immediately for low latency.
    pub fn write_chunk(&self, s16le_data: Vec<u8>, stream_id: u64) -> Result<(), AudioError> {
        self.command_tx
            .send(AudioCommand::WriteChunk {
                data: s16le_data,
                stream_id,
            })
            .map_err(|_| AudioError::WriteError("Audio thread disconnected".to_string()))?;
        Ok(())
    }

    /// Signal end of stream and wait for completion (blocking).
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

    /// Signal end of stream (non-blocking) -- returns a receiver to check completion.
    pub fn end_stream(&self) -> Result<mpsc::Receiver<()>, AudioError> {
        let (completion_tx, completion_rx) = mpsc::channel();
        self.command_tx
            .send(AudioCommand::EndStreamAndWait(completion_tx))
            .map_err(|_| AudioError::WriteError("Audio thread disconnected".to_string()))?;
        Ok(completion_rx)
    }

    /// Abort current playback immediately.
    pub fn abort(&self) -> Result<(), AudioError> {
        self.command_tx
            .send(AudioCommand::Abort)
            .map_err(|_| AudioError::WriteError("Audio thread disconnected".to_string()))?;
        Ok(())
    }

    fn run_cpal_thread(
        command_rx: Receiver<AudioCommand>,
        config: AudioSinkConfig,
        ready_tx: mpsc::SyncSender<Result<(), AudioError>>,
    ) -> Result<(), AudioError> {
        let host = cpal::default_host();

        macro_rules! bail {
            ($e:expr) => {{
                let err: AudioError = $e;
                let _ = ready_tx.send(Err(err.clone()));
                return Err(err);
            }};
        }

        let device = if let Some(name) = &config.device_name {
            log::info!("AudioSink: Available output devices:");
            let mut found_device = None;
            match host.output_devices() {
                Ok(devices) => {
                    for device in devices {
                        match device.name() {
                            Ok(device_name) => {
                                log::info!("  - {}", device_name);
                                if device_name == *name {
                                    found_device = Some(device);
                                }
                            }
                            Err(e) => bail!(AudioError::from(e)),
                        }
                    }
                }
                Err(e) => bail!(AudioError::from(e)),
            }
            match found_device {
                Some(d) => d,
                None => bail!(AudioError::DeviceError(format!(
                    "Output device '{}' not found",
                    name
                ))),
            }
        } else {
            match host.default_output_device() {
                Some(d) => d,
                None => bail!(AudioError::DeviceError(
                    "No output device available".to_string()
                )),
            }
        };

        log::info!("🔊 Using output device: {:?}", device.name());

        let supported_config = Self::select_output_config(&device).unwrap_or_else(|e| {
            log::warn!(
                "⚠️  Preferred output config not found ({}), using device default",
                e
            );
            device.default_output_config().expect("no output config")
        });

        let stream_config = supported_config.config();
        let hardware_sample_rate = stream_config.sample_rate.0;
        let hardware_channels = stream_config.channels;

        log::info!(
            "🔊 Output: {}Hz, {}ch, {:?}",
            hardware_sample_rate,
            hardware_channels,
            supported_config.sample_format(),
        );

        let audio_buffer: Arc<Mutex<Vec<i16>>> = Arc::new(Mutex::new(Vec::new()));

        let stream = Self::build_i16_stream(&device, &stream_config, Arc::clone(&audio_buffer))
            .map_err(|e| {
                let _ = ready_tx.send(Err(e.clone()));
                e
            })?;

        stream.play().map_err(|e| {
            let err = AudioError::from(e);
            let _ = ready_tx.send(Err(err.clone()));
            err
        })?;

        let _ = ready_tx.send(Ok(()));

        let mut completion_signals: Vec<mpsc::Sender<()>> = Vec::new();
        let mut current_stream_id: u64 = 0;

        loop {
            match command_rx.recv_timeout(Duration::from_millis(10)) {
                Ok(command) => {
                    match command {
                        AudioCommand::WriteChunk {
                            data: s16le_data,
                            stream_id,
                        } => {
                            if stream_id != current_stream_id {
                                if current_stream_id != 0 {
                                    log::info!(
                                        "🔄 Stream switch: {} → {} (dropping old audio)",
                                        current_stream_id,
                                        stream_id
                                    );

                                    let mut drained_from_queue = 0;
                                    while let Ok(cmd) = command_rx.try_recv() {
                                        match cmd {
                                            AudioCommand::WriteChunk {
                                                stream_id: cmd_sid,
                                                ..
                                            } => {
                                                if cmd_sid == current_stream_id {
                                                    drained_from_queue += 1;
                                                } else {
                                                    break;
                                                }
                                            }
                                            AudioCommand::EndStreamAndWait(tx) => {
                                                completion_signals.push(tx);
                                            }
                                            AudioCommand::Abort => {}
                                        }
                                    }

                                    {
                                        let mut buffer = audio_buffer.lock().unwrap();
                                        buffer.clear();
                                    }

                                    if drained_from_queue > 0 {
                                        log::info!(
                                            "🗑️  Dropped {} old chunks from stream {}",
                                            drained_from_queue,
                                            current_stream_id
                                        );
                                    }

                                    for tx in completion_signals.drain(..) {
                                        let _ = tx.send(());
                                    }
                                } else {
                                    log::info!("🆕 First stream started: {}", stream_id);
                                }
                                current_stream_id = stream_id;
                            }

                            {
                                let mut buffer = audio_buffer.lock().unwrap();
                                extend_buffer_from_s16le(&mut buffer, &s16le_data)?;
                            }

                            if !completion_signals.is_empty() {
                                let queue_is_empty = command_rx.is_empty();
                                let buffer_len = {
                                    let buffer = audio_buffer.lock().unwrap();
                                    buffer.len()
                                };

                                if queue_is_empty
                                    && buffer_len < hardware_sample_rate as usize / 50
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
                            let mut drained_count = 0;
                            while let Ok(cmd) = command_rx.try_recv() {
                                match cmd {
                                    AudioCommand::WriteChunk { .. } => {
                                        drained_count += 1;
                                    }
                                    AudioCommand::EndStreamAndWait(tx) => {
                                        completion_signals.push(tx);
                                    }
                                    AudioCommand::Abort => {}
                                }
                            }

                            if drained_count > 0 {
                                log::info!(
                                    "🗑️  Drained {} buffered audio chunks during abort",
                                    drained_count
                                );
                            }

                            {
                                let mut buffer = audio_buffer.lock().unwrap();
                                buffer.clear();
                            }

                            current_stream_id = 0;

                            for tx in completion_signals.drain(..) {
                                let _ = tx.send(());
                            }
                        }
                    }
                }
                Err(crossbeam::channel::RecvTimeoutError::Timeout) => {
                    if !completion_signals.is_empty() {
                        let queue_is_empty = command_rx.is_empty();
                        let buffer_len = {
                            let buffer = audio_buffer.lock().unwrap();
                            buffer.len()
                        };

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

    /// Find best I16 output config at 48kHz mono.
    fn select_output_config(
        device: &cpal::Device,
    ) -> Result<cpal::SupportedStreamConfig, AudioError> {
        let configs = device
            .supported_output_configs()
            .map_err(|e| AudioError::DeviceError(e.to_string()))?;

        let mut best: Option<cpal::SupportedStreamConfig> = None;
        let mut best_score = u32::MAX;

        for range in configs {
            if range.channels() != TARGET_CHANNELS {
                continue;
            }
            let format_penalty: u32 = if range.sample_format() == SampleFormat::I16 {
                0
            } else {
                100
            };

            let min = range.min_sample_rate().0;
            let max = range.max_sample_rate().0;
            let rate = TARGET_SAMPLE_RATE.clamp(min, max);
            let rate_penalty = rate.abs_diff(TARGET_SAMPLE_RATE);

            let score = format_penalty + rate_penalty;
            if score < best_score {
                best_score = score;
                best = Some(range.with_sample_rate(cpal::SampleRate(rate)));
            }
        }

        best.ok_or_else(|| {
            AudioError::DeviceError("No matching output config found".to_string())
        })
    }

    fn build_i16_stream(
        device: &cpal::Device,
        config: &cpal::StreamConfig,
        audio_buffer: Arc<Mutex<Vec<i16>>>,
    ) -> Result<Stream, AudioError> {
        device
            .build_output_stream(
                config,
                move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                    let mut buffer = audio_buffer.lock().unwrap();
                    let (samples, underrun) = drain_samples(&mut buffer, data.len());

                    let copy_len = samples.len().min(data.len());
                    data[..copy_len].copy_from_slice(&samples[..copy_len]);

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_s16le_buffer_passthrough() {
        let s16le_data = vec![
            0x00, 0x10, // 4096
            0x00, 0x20, // 8192
            0x00, 0x30, // 12288
            0x00, 0x40, // 16384
        ];

        let mut buffer = Vec::new();
        extend_buffer_from_s16le(&mut buffer, &s16le_data).unwrap();

        assert_eq!(buffer.len(), 4);
        assert_eq!(buffer[0], 4096);
        assert_eq!(buffer[1], 8192);
        assert_eq!(buffer[2], 12288);
        assert_eq!(buffer[3], 16384);
    }

    #[test]
    fn test_buffer_underrun_handling() {
        let mut buffer = vec![1i16, 2, 3];

        let (samples, underrun) = drain_samples(&mut buffer, 5);

        assert_eq!(samples.len(), 3);
        assert!(underrun);
        assert_eq!(buffer.len(), 0);
    }
}
