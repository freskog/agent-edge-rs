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
const PREFILL_SILENCE_SAMPLES: usize = 9600; // 200ms at 48kHz

/// Minimum number of buffered samples before we start CONSUMING from the
/// ring on a new stream. The cpal callback emits silence while priming
/// (without draining the ring), so the bursty TCP producer gets time to
/// build head-room. This is an *upper bound* on added latency: in the fast
/// case the watermark is reached almost immediately, in the slow case we
/// wait at most this much before unmuting. It's tunable at runtime via the
/// AUDIO_PRIME_MS env var (in milliseconds, default 500).
const STREAM_PRIME_DEFAULT_MS: u64 = 500;
const STREAM_PRIME_MIN_MS: u64 = 50;
const STREAM_PRIME_MAX_MS: u64 = 2000;

fn stream_prime_samples() -> usize {
    let ms = std::env::var("AUDIO_PRIME_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(STREAM_PRIME_DEFAULT_MS)
        .clamp(STREAM_PRIME_MIN_MS, STREAM_PRIME_MAX_MS);
    (ms as usize) * (TARGET_SAMPLE_RATE as usize) / 1000
}

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

/// Lock-free diagnostics shared with the cpal output callback.
/// All fields are atomics that the callback updates and the audio_sink loop
/// drains every few seconds.
#[derive(Clone)]
struct StreamDiagnostics {
    cb_count: Arc<AtomicU64>,
    cb_period_max_us: Arc<AtomicU64>,
    cb_period_min_us: Arc<AtomicU64>,
    cb_work_max_us: Arc<AtomicU64>,
    cb_buf_min: Arc<AtomicU64>,
    cb_buf_max: Arc<AtomicU64>,
    cb_alsa_delay_min_us: Arc<AtomicU64>,
    cb_alsa_delay_max_us: Arc<AtomicU64>,
    cb_ring_min: Arc<AtomicU64>,
    cb_ring_max: Arc<AtomicU64>,
    cb_silent_pad_count: Arc<AtomicU64>,
    cb_silent_pad_samples: Arc<AtomicU64>,
}

#[inline]
fn atomic_min_u64(slot: &AtomicU64, val: u64) {
    let mut cur = slot.load(Ordering::Relaxed);
    while val < cur {
        match slot.compare_exchange_weak(cur, val, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(observed) => cur = observed,
        }
    }
}

#[inline]
fn atomic_max_u64(slot: &AtomicU64, val: u64) {
    let mut cur = slot.load(Ordering::Relaxed);
    while val > cur {
        match slot.compare_exchange_weak(cur, val, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(observed) => cur = observed,
        }
    }
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
        // Per-interval callback diagnostics (lock-free, lightweight).
        let cb_count = Arc::new(AtomicU64::new(0));
        let cb_period_max_us = Arc::new(AtomicU64::new(0));
        let cb_period_min_us = Arc::new(AtomicU64::new(u64::MAX));
        let cb_work_max_us = Arc::new(AtomicU64::new(0));
        let cb_buf_min = Arc::new(AtomicU64::new(u64::MAX));
        let cb_buf_max = Arc::new(AtomicU64::new(0));
        let cb_alsa_delay_min_us = Arc::new(AtomicU64::new(u64::MAX));
        let cb_alsa_delay_max_us = Arc::new(AtomicU64::new(0));
        let cb_ring_min = Arc::new(AtomicU64::new(u64::MAX));
        let cb_ring_max = Arc::new(AtomicU64::new(0));
        let cb_silent_pad_count = Arc::new(AtomicU64::new(0));
        let cb_silent_pad_samples = Arc::new(AtomicU64::new(0));

        let rb = HeapRb::<i16>::new(RING_CAPACITY);
        let (mut prod, cons) = rb.split();

        let silence = vec![0i16; PREFILL_SILENCE_SAMPLES];
        prod.push_slice(&silence);

        let stream_diag = StreamDiagnostics {
            cb_count: Arc::clone(&cb_count),
            cb_period_max_us: Arc::clone(&cb_period_max_us),
            cb_period_min_us: Arc::clone(&cb_period_min_us),
            cb_work_max_us: Arc::clone(&cb_work_max_us),
            cb_buf_min: Arc::clone(&cb_buf_min),
            cb_buf_max: Arc::clone(&cb_buf_max),
            cb_alsa_delay_min_us: Arc::clone(&cb_alsa_delay_min_us),
            cb_alsa_delay_max_us: Arc::clone(&cb_alsa_delay_max_us),
            cb_ring_min: Arc::clone(&cb_ring_min),
            cb_ring_max: Arc::clone(&cb_ring_max),
            cb_silent_pad_count: Arc::clone(&cb_silent_pad_count),
            cb_silent_pad_samples: Arc::clone(&cb_silent_pad_samples),
        };

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
            stream_diag.clone(),
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
        let prime_target = stream_prime_samples();
        log::info!(
            "🎯 Stream prime watermark: {} samples (~{}ms)",
            prime_target,
            prime_target * 1000 / TARGET_SAMPLE_RATE as usize
        );

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
                    stream_diag.clone(),
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

            if last_underrun_log.elapsed() >= Duration::from_secs(2) {
                let count = underrun_count.swap(0, Ordering::Relaxed);
                let cb_n = cb_count.swap(0, Ordering::Relaxed);
                let pmax = cb_period_max_us.swap(0, Ordering::Relaxed);
                let pmin = cb_period_min_us.swap(u64::MAX, Ordering::Relaxed);
                let work = cb_work_max_us.swap(0, Ordering::Relaxed);
                let bmin = cb_buf_min.swap(u64::MAX, Ordering::Relaxed);
                let bmax = cb_buf_max.swap(0, Ordering::Relaxed);
                let dmin = cb_alsa_delay_min_us.swap(u64::MAX, Ordering::Relaxed);
                let dmax = cb_alsa_delay_max_us.swap(0, Ordering::Relaxed);
                let rmin = cb_ring_min.swap(u64::MAX, Ordering::Relaxed);
                let rmax = cb_ring_max.swap(0, Ordering::Relaxed);
                let pad_n = cb_silent_pad_count.swap(0, Ordering::Relaxed);
                let pad_s = cb_silent_pad_samples.swap(0, Ordering::Relaxed);

                if playback_active.load(Ordering::Relaxed) || count > 0 || pad_n > 0 {
                    let pmin_show = if pmin == u64::MAX { 0 } else { pmin };
                    let dmin_show = if dmin == u64::MAX { 0 } else { dmin };
                    let bmin_show = if bmin == u64::MAX { 0 } else { bmin };
                    let rmin_show = if rmin == u64::MAX { 0 } else { rmin };
                    log::info!(
                        "📊 cb={} period_us=[{}..{}] work_us={} buf=[{}..{}] alsa_delay_us=[{}..{}] ring=[{}..{}] underruns={} pad_events={} pad_samples={}",
                        cb_n, pmin_show, pmax, work,
                        bmin_show, bmax,
                        dmin_show, dmax,
                        rmin_show, rmax,
                        count, pad_n, pad_s
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
                            // Chunks belonging to the new stream that were already queued
                            // behind `s16le_data` when the stream switch was detected. They
                            // arrived AFTER `s16le_data`, so they must be pushed AFTER it
                            // to preserve playback order.
                            let mut saved_new_chunks: Vec<Vec<u8>> = Vec::new();

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
                                                data,
                                                stream_id: cmd_sid,
                                            } => {
                                                if cmd_sid == current_stream_id {
                                                    drained_from_queue += 1;
                                                } else if cmd_sid == stream_id {
                                                    saved_new_chunks.push(data);
                                                } else {
                                                    // Yet another newer stream id — discard
                                                    // (rare, but keeps us from getting stuck).
                                                    drained_from_queue += 1;
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
                                } else {
                                    log::info!("🆕 First stream started: {}", stream_id);
                                }
                                current_stream_id = stream_id;
                                stream_chunk_count = 0;
                                stream_total_bytes = 0;
                                stream_total_samples = 0;
                                stream_start_time = Some(Instant::now());
                                callback_samples_consumed.store(0, Ordering::Relaxed);

                                // Enter "priming" phase: cpal callback emits silence
                                // without draining the ring while we build head-room.
                                // Will be flipped to true below once the ring has
                                // accumulated `prime_target` samples (or the stream
                                // ends, whichever comes first).
                                playback_active.store(false, Ordering::Release);
                            }

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

                            // Push chunks that were queued behind the current one (in order).
                            for chunk in saved_new_chunks.drain(..) {
                                let pushed2 =
                                    push_s16le_to_ring(&mut prod, &chunk)?;
                                stream_chunk_count += 1;
                                stream_total_bytes += chunk.len() as u64;
                                stream_total_samples += pushed2 as u64;
                            }

                            // If we're priming and the ring has reached the
                            // watermark, release playback. The cpal callback
                            // will now start consuming real audio with enough
                            // head-room to ride out the producer's bursts.
                            if !playback_active.load(Ordering::Acquire)
                                && prod.occupied_len() >= prime_target
                            {
                                let buffered = prod.occupied_len();
                                let prime_ms = stream_start_time
                                    .map(|t| t.elapsed().as_millis())
                                    .unwrap_or(0);
                                log::info!(
                                    "🎬 Stream {} primed: {} samples buffered (~{}ms audio), priming took {}ms wall",
                                    current_stream_id,
                                    buffered,
                                    buffered * 1000 / TARGET_SAMPLE_RATE as usize,
                                    prime_ms
                                );
                                playback_active.store(true, Ordering::Release);
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
                                    current_stream_id = 0;
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
                            // Stream ended before reaching the priming watermark
                            // (a short utterance). Release playback now so the
                            // buffered audio actually drains and we can signal
                            // completion to the producer.
                            if !playback_active.load(Ordering::Acquire) && ring_occ > 0 {
                                log::info!(
                                    "🎬 Stream {} ending below prime watermark, releasing playback with {} samples (~{}ms)",
                                    current_stream_id,
                                    ring_occ,
                                    ring_occ * 1000 / TARGET_SAMPLE_RATE as usize
                                );
                                playback_active.store(true, Ordering::Release);
                            }
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
                            current_stream_id = 0;
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
        diag: StreamDiagnostics,
    ) -> Result<Stream, AudioError> {
        let channels = config.channels as usize;
        let rt_set = Arc::new(AtomicBool::new(false));
        // Per-callback timing state (only touched by the callback thread).
        let mut last_cb_instant: Option<Instant> = None;
        device
            .build_output_stream(
                config,
                move |data: &mut [i16], info: &cpal::OutputCallbackInfo| {
                    let cb_entry = Instant::now();

                    #[cfg(target_os = "linux")]
                    if !rt_set.load(Ordering::Relaxed) {
                        rt_set.store(true, Ordering::Relaxed);
                        unsafe {
                            let param = libc::sched_param { sched_priority: 10 };
                            let ret = libc::sched_setscheduler(0, libc::SCHED_FIFO, &param);
                            if ret == 0 {
                                log::info!("Audio callback thread set to SCHED_FIFO priority 10");
                            } else {
                                log::warn!(
                                    "Failed to set RT on callback thread: {}",
                                    std::io::Error::last_os_error()
                                );
                            }
                            let mut cpuset: libc::cpu_set_t = std::mem::zeroed();
                            libc::CPU_SET(0, &mut cpuset);
                            let ret = libc::sched_setaffinity(
                                0,
                                std::mem::size_of::<libc::cpu_set_t>(),
                                &cpuset,
                            );
                            if ret == 0 {
                                log::info!("Audio callback thread pinned to core 0");
                            } else {
                                log::warn!(
                                    "Failed to pin callback thread to core 0: {}",
                                    std::io::Error::last_os_error()
                                );
                            }
                        }
                    }

                    // Inter-callback period (jitter)
                    if let Some(prev) = last_cb_instant {
                        let period_us = cb_entry.duration_since(prev).as_micros() as u64;
                        atomic_max_u64(&diag.cb_period_max_us, period_us);
                        atomic_min_u64(&diag.cb_period_min_us, period_us);
                    }
                    last_cb_instant = Some(cb_entry);

                    // ALSA-reported time between this callback firing and when
                    // the audio will hit the DAC. Sudden drops indicate xruns
                    // / late callbacks; growth indicates we're filling further
                    // ahead than usual.
                    let ts = info.timestamp();
                    if let Some(delay) = ts.playback.duration_since(&ts.callback) {
                        let delay_us = delay.as_micros() as u64;
                        atomic_max_u64(&diag.cb_alsa_delay_max_us, delay_us);
                        atomic_min_u64(&diag.cb_alsa_delay_min_us, delay_us);
                    }

                    let buf_len = data.len() as u64;
                    callback_buf_size.store(buf_len, Ordering::Relaxed);
                    atomic_max_u64(&diag.cb_buf_max, buf_len);
                    atomic_min_u64(&diag.cb_buf_min, buf_len);
                    diag.cb_count.fetch_add(1, Ordering::Relaxed);

                    if clear_flag.load(Ordering::Acquire) {
                        consumer.clear();
                        clear_flag.store(false, Ordering::Release);
                        for s in data.iter_mut() {
                            *s = 0;
                        }
                        let work_us = cb_entry.elapsed().as_micros() as u64;
                        atomic_max_u64(&diag.cb_work_max_us, work_us);
                        return;
                    }

                    let ring_before = consumer.occupied_len() as u64;
                    atomic_max_u64(&diag.cb_ring_max, ring_before);
                    atomic_min_u64(&diag.cb_ring_min, ring_before);

                    // Priming phase: emit silence without consuming from the
                    // ring so the producer can build head-room. The audio_sink
                    // loop flips playback_active to true when the watermark is
                    // reached or the stream ends.
                    if !playback_active.load(Ordering::Relaxed) {
                        for s in data.iter_mut() {
                            *s = 0;
                        }
                        let work_us = cb_entry.elapsed().as_micros() as u64;
                        atomic_max_u64(&diag.cb_work_max_us, work_us);
                        return;
                    }

                    if channels == 1 {
                        let popped = consumer.pop_slice(data);
                        if popped > 0 {
                            callback_samples_consumed
                                .fetch_add(popped as u64, Ordering::Relaxed);
                        }
                        if popped < data.len() {
                            let pad = (data.len() - popped) as u64;
                            for s in data[popped..].iter_mut() {
                                *s = 0;
                            }
                            if playback_active.load(Ordering::Relaxed) {
                                underrun_count.fetch_add(1, Ordering::Relaxed);
                                diag.cb_silent_pad_count.fetch_add(1, Ordering::Relaxed);
                                diag.cb_silent_pad_samples
                                    .fetch_add(pad, Ordering::Relaxed);
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
                            let pad = (frames - popped) as u64;
                            for s in data[popped * channels..].iter_mut() {
                                *s = 0;
                            }
                            if playback_active.load(Ordering::Relaxed) {
                                underrun_count.fetch_add(1, Ordering::Relaxed);
                                diag.cb_silent_pad_count.fetch_add(1, Ordering::Relaxed);
                                diag.cb_silent_pad_samples
                                    .fetch_add(pad, Ordering::Relaxed);
                            }
                        }
                    }

                    let work_us = cb_entry.elapsed().as_micros() as u64;
                    atomic_max_u64(&diag.cb_work_max_us, work_us);
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
