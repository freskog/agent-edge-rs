use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{
    BuildStreamError, DeviceNameError, DevicesError, PlayStreamError, SampleFormat, Stream,
    SupportedStreamConfigsError,
};
use crossbeam::channel::{bounded, Receiver, Sender};
use ringbuf::{
    traits::{Consumer as RbConsumer, Observer as RbObserver, Producer as RbProducer, Split},
    HeapCons, HeapProd, HeapRb,
};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};
use thiserror::Error;

pub use crate::types::AudioDeviceInfo;

const TARGET_SAMPLE_RATE: u32 = 48000;
const TARGET_CHANNELS: u16 = 1;
const RING_CAPACITY: usize = 240_000; // 5 seconds at 48kHz
const MAX_CALLBACK_FRAMES: usize = 8192;
/// Silence to pre-fill the ring buffer before stream.play() so the first
/// ALSA period has data and doesn't immediately underrun.
const PREFILL_SILENCE_SAMPLES: usize = 4800; // 100ms at 48kHz

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

/// Decode s16le bytes and push into the SPSC ring buffer (command thread only).
/// Spins with short sleeps when the ring is full so we never silently drop samples.
/// Returns the number of i16 samples pushed.
fn push_s16le_to_ring(prod: &mut HeapProd<i16>, s16le_data: &[u8]) -> Result<usize, AudioError> {
    if s16le_data.len() % 2 != 0 {
        return Err(AudioError::WriteError(
            "S16LE data length not aligned to 16-bit samples".to_string(),
        ));
    }
    let samples: Vec<i16> = s16le_data
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect();

    let total = samples.len();
    let mut offset = 0;
    let mut spins = 0u32;
    while offset < total {
        let pushed = prod.push_slice(&samples[offset..]);
        offset += pushed;
        if offset < total {
            spins += 1;
            thread::sleep(Duration::from_millis(1));
        }
    }
    if spins > 0 {
        log::debug!(
            "🔁 push_s16le_to_ring: ring was full, spun {}ms to push {} samples",
            spins,
            total
        );
    }
    Ok(total)
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

        let underrun_count = Arc::new(AtomicU64::new(0));
        let stream_broken = Arc::new(AtomicBool::new(false));
        let clear_flag = Arc::new(AtomicBool::new(false));
        let playback_active = Arc::new(AtomicBool::new(false));
        let callback_samples_consumed = Arc::new(AtomicU64::new(0));
        let callback_buf_size = Arc::new(AtomicU64::new(0));

        let rb = HeapRb::<i16>::new(RING_CAPACITY);
        let (mut prod, cons) = rb.split();

        let silence = vec![0i16; PREFILL_SILENCE_SAMPLES];
        prod.push_slice(&silence);

        let mut stream = Self::build_i16_stream(
            &device,
            &stream_config,
            cons,
            Arc::clone(&underrun_count),
            Arc::clone(&stream_broken),
            Arc::clone(&clear_flag),
            Arc::clone(&playback_active),
            Arc::clone(&callback_samples_consumed),
            Arc::clone(&callback_buf_size),
        )
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
        let mut last_underrun_log = Instant::now();
        let mut last_recreation = Instant::now();
        let mut stream_chunk_count: u64 = 0;
        let mut stream_total_bytes: u64 = 0;
        let mut stream_total_samples: u64 = 0;
        let mut stream_start_time: Option<Instant> = None;

        loop {
            if stream_broken.load(Ordering::Acquire)
                && last_recreation.elapsed() >= Duration::from_millis(500)
            {
                log::warn!("CPAL stream broken (xrun), recreating...");
                drop(stream);

                let rb = HeapRb::<i16>::new(RING_CAPACITY);
                let (new_prod, new_cons) = rb.split();
                prod = new_prod;

                let silence = vec![0i16; PREFILL_SILENCE_SAMPLES];
                prod.push_slice(&silence);

                stream_broken.store(false, Ordering::Release);

                match Self::build_i16_stream(
                    &device,
                    &stream_config,
                    new_cons,
                    Arc::clone(&underrun_count),
                    Arc::clone(&stream_broken),
                    Arc::clone(&clear_flag),
                    Arc::clone(&playback_active),
                    Arc::clone(&callback_samples_consumed),
                    Arc::clone(&callback_buf_size),
                ) {
                    Ok(s) => {
                        if let Err(e) = s.play() {
                            log::error!("Failed to restart stream after xrun: {}", e);
                            break;
                        }
                        stream = s;
                        last_recreation = Instant::now();
                        log::info!("Stream recreated successfully after xrun");
                    }
                    Err(e) => {
                        log::error!("Failed to recreate stream after xrun: {}", e);
                        break;
                    }
                }
            }

            if last_underrun_log.elapsed() >= Duration::from_secs(5) {
                let count = underrun_count.swap(0, Ordering::Relaxed);
                let consumed = callback_samples_consumed.load(Ordering::Relaxed);
                let buf_sz = callback_buf_size.load(Ordering::Relaxed);
                let ring_occ = prod.occupied_len();
                if count > 0 {
                    log::warn!(
                        "Playback underruns in last 5s: {} | ring={}/{} cb_consumed={} cb_buf={}",
                        count, ring_occ, RING_CAPACITY, consumed, buf_sz
                    );
                }
                last_underrun_log = Instant::now();
            }

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
                                    let mut saved_new_chunk: Option<Vec<u8>> = None;
                                    while let Ok(cmd) = command_rx.try_recv() {
                                        match cmd {
                                            AudioCommand::WriteChunk {
                                                data,
                                                stream_id: cmd_sid,
                                            } => {
                                                if cmd_sid == current_stream_id {
                                                    drained_from_queue += 1;
                                                } else {
                                                    saved_new_chunk = Some(data);
                                                    break;
                                                }
                                            }
                                            AudioCommand::EndStreamAndWait(tx) => {
                                                completion_signals.push(tx);
                                            }
                                            AudioCommand::Abort => {}
                                        }
                                    }

                                    clear_flag.store(true, Ordering::Release);

                                    // Wait for the callback to process the clear so we don't
                                    // push new-stream data that immediately gets wiped.
                                    let clear_deadline =
                                        Instant::now() + Duration::from_millis(50);
                                    while clear_flag.load(Ordering::Acquire)
                                        && Instant::now() < clear_deadline
                                    {
                                        thread::sleep(Duration::from_millis(1));
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

                                    // Push any new-stream chunk that was consumed during drain
                                    if let Some(chunk) = saved_new_chunk {
                                        push_s16le_to_ring(&mut prod, &chunk)?;
                                    }
                                } else {
                                    log::info!("🆕 First stream started: {}", stream_id);
                                }
                                current_stream_id = stream_id;
                                stream_chunk_count = 0;
                                stream_total_bytes = 0;
                                stream_total_samples = 0;
                                stream_start_time = Some(Instant::now());
                                callback_samples_consumed.store(0, Ordering::Relaxed);
                            }

                            playback_active.store(true, Ordering::Release);
                            let pushed =
                                push_s16le_to_ring(&mut prod, &s16le_data)?;
                            stream_chunk_count += 1;
                            stream_total_bytes += s16le_data.len() as u64;
                            stream_total_samples += pushed as u64;
                            if stream_chunk_count == 1 {
                                log::info!(
                                    "📦 Stream {} chunk #1: {} bytes, {} samples, ring={}",
                                    stream_id,
                                    s16le_data.len(),
                                    pushed,
                                    prod.occupied_len()
                                );
                            }

                            if !completion_signals.is_empty() {
                                let queue_is_empty = command_rx.is_empty();
                                let buffer_len = prod.occupied_len();

                                if queue_is_empty && buffer_len == 0 {
                                    let elapsed = stream_start_time
                                        .map(|t| t.elapsed().as_millis())
                                        .unwrap_or(0);
                                    let consumed =
                                        callback_samples_consumed.load(Ordering::Relaxed);
                                    log::info!(
                                        "✅ Stream {} complete: {} chunks, {} bytes, {} samples pushed, {} consumed by cb, {:.1}s audio, {}ms wall",
                                        current_stream_id,
                                        stream_chunk_count,
                                        stream_total_bytes,
                                        stream_total_samples,
                                        consumed,
                                        stream_total_samples as f64 / TARGET_SAMPLE_RATE as f64,
                                        elapsed
                                    );
                                    playback_active.store(false, Ordering::Release);
                                    for tx in completion_signals.drain(..) {
                                        let _ = tx.send(());
                                    }
                                }
                            }
                        }
                        AudioCommand::EndStreamAndWait(tx) => {
                            let ring_occ = prod.occupied_len();
                            log::info!(
                                "🏁 EndStreamAndWait for stream {}: {} chunks, {} samples pushed, ring={}, cb_consumed={}",
                                current_stream_id,
                                stream_chunk_count,
                                stream_total_samples,
                                ring_occ,
                                callback_samples_consumed.load(Ordering::Relaxed)
                            );
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

                            clear_flag.store(true, Ordering::Release);
                            playback_active.store(false, Ordering::Release);

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
                        let buffer_len = prod.occupied_len();

                        if queue_is_empty && buffer_len == 0 {
                            let elapsed = stream_start_time
                                .map(|t| t.elapsed().as_millis())
                                .unwrap_or(0);
                            let consumed =
                                callback_samples_consumed.load(Ordering::Relaxed);
                            log::info!(
                                "✅ Stream {} complete: {} chunks, {} bytes, {} samples pushed, {} consumed by cb, {:.1}s audio, {}ms wall",
                                current_stream_id,
                                stream_chunk_count,
                                stream_total_bytes,
                                stream_total_samples,
                                consumed,
                                stream_total_samples as f64 / TARGET_SAMPLE_RATE as f64,
                                elapsed
                            );
                            playback_active.store(false, Ordering::Release);
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

    /// Find best I16 output config at 48kHz, preferring mono but accepting stereo.
    fn select_output_config(
        device: &cpal::Device,
    ) -> Result<cpal::SupportedStreamConfig, AudioError> {
        let configs = device
            .supported_output_configs()
            .map_err(|e| AudioError::DeviceError(e.to_string()))?;

        let mut best: Option<cpal::SupportedStreamConfig> = None;
        let mut best_score = u32::MAX;

        for range in configs {
            let channel_penalty: u32 = if range.channels() == TARGET_CHANNELS {
                0
            } else if range.channels() == 2 {
                10
            } else {
                continue;
            };

            let format_penalty: u32 = if range.sample_format() == SampleFormat::I16 {
                0
            } else {
                100
            };

            let min = range.min_sample_rate().0;
            let max = range.max_sample_rate().0;
            let rate = TARGET_SAMPLE_RATE.clamp(min, max);
            let rate_penalty = rate.abs_diff(TARGET_SAMPLE_RATE);

            let score = format_penalty + rate_penalty + channel_penalty;
            if score < best_score {
                best_score = score;
                best = Some(range.with_sample_rate(cpal::SampleRate(rate)));
            }
        }

        best.ok_or_else(|| {
            AudioError::DeviceError("No matching output config found".to_string())
        })
    }

    /// Build the CPAL output stream with a lock-free ring buffer consumer.
    /// The callback is allocation-free and mutex-free.
    fn build_i16_stream(
        device: &cpal::Device,
        config: &cpal::StreamConfig,
        mut consumer: HeapCons<i16>,
        underrun_count: Arc<AtomicU64>,
        stream_broken: Arc<AtomicBool>,
        clear_flag: Arc<AtomicBool>,
        playback_active: Arc<AtomicBool>,
        callback_samples_consumed: Arc<AtomicU64>,
        callback_buf_size: Arc<AtomicU64>,
    ) -> Result<Stream, AudioError> {
        let channels = config.channels as usize;
        device
            .build_output_stream(
                config,
                move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                    callback_buf_size.store(data.len() as u64, Ordering::Relaxed);

                    if clear_flag.load(Ordering::Acquire) {
                        consumer.clear();
                        clear_flag.store(false, Ordering::Release);
                        for s in data.iter_mut() {
                            *s = 0;
                        }
                        return;
                    }

                    if channels == 1 {
                        let popped = consumer.pop_slice(data);
                        if popped > 0 {
                            callback_samples_consumed
                                .fetch_add(popped as u64, Ordering::Relaxed);
                        }
                        if popped < data.len() {
                            for s in data[popped..].iter_mut() {
                                *s = 0;
                            }
                            if playback_active.load(Ordering::Relaxed) {
                                underrun_count.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    } else {
                        let frames = data.len() / channels;
                        let mut mono_buf = [0i16; MAX_CALLBACK_FRAMES];
                        let to_pop = frames.min(MAX_CALLBACK_FRAMES);
                        let popped = consumer.pop_slice(&mut mono_buf[..to_pop]);
                        if popped > 0 {
                            callback_samples_consumed
                                .fetch_add(popped as u64, Ordering::Relaxed);
                        }
                        for i in 0..popped {
                            for ch in 0..channels {
                                data[i * channels + ch] = mono_buf[i];
                            }
                        }
                        if popped < frames {
                            for s in data[popped * channels..].iter_mut() {
                                *s = 0;
                            }
                            if playback_active.load(Ordering::Relaxed) {
                                underrun_count.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                },
                move |err| {
                    if stream_broken
                        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
                        .is_ok()
                    {
                        log::error!("CPAL stream error: {}", err);
                    }
                },
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
    fn test_s16le_ring_push() {
        let s16le_data = vec![
            0x00, 0x10, // 4096
            0x00, 0x20, // 8192
            0x00, 0x30, // 12288
            0x00, 0x40, // 16384
        ];

        let rb = HeapRb::<i16>::new(64);
        let (mut prod, mut cons) = rb.split();
        push_s16le_to_ring(&mut prod, &s16le_data).unwrap();

        assert_eq!(prod.occupied_len(), 4);

        let mut out = [0i16; 4];
        let n = cons.pop_slice(&mut out);
        assert_eq!(n, 4);
        assert_eq!(out, [4096, 8192, 12288, 16384]);
    }

    #[test]
    fn test_ring_underrun_produces_fewer_samples() {
        let rb = HeapRb::<i16>::new(64);
        let (mut prod, mut cons) = rb.split();

        prod.push_slice(&[1i16, 2, 3]);

        let mut out = [0i16; 5];
        let n = cons.pop_slice(&mut out);
        assert_eq!(n, 3);
        assert_eq!(out[..3], [1, 2, 3]);
        assert_eq!(prod.occupied_len(), 0);
    }
}
