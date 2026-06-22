use crate::audio_source::{AudioCapture, AudioCaptureConfig};
use crate::mpv_controller::MpvController;
use crate::protocol::{ConsumerConnection, ConsumerMessage, ProtocolError};
use crate::spotify_controller::SpotifyController;
use crate::wakeword_model::Model as WakewordModel;
use crate::wakeword_vad::{VadConfig, VadProcessor};
use crossbeam::channel::{Receiver, Sender};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConsumerServerError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Protocol error: {0}")]
    Protocol(#[from] ProtocolError),

    #[error("Audio error: {0}")]
    Audio(String),

    #[error("Consumer already connected")]
    ConsumerAlreadyConnected,
}

/// Paired audio chunk with detection results
#[derive(Debug, Clone)]
pub struct AudioDetectionPair {
    pub audio_data: Vec<u8>,
    pub speech_detected: bool,
    pub wakeword_event: Option<WakewordEvent>,
    pub timestamp: u64,
}

/// Wakeword detection event
#[derive(Debug, Clone)]
pub struct WakewordEvent {
    pub model: String,
    pub confidence: f32,
    pub timestamp: u64,
    pub spotify_was_paused: bool,
    pub mpv_was_paused: bool,
}

/// Configuration for the consumer server
#[derive(Clone)]
pub struct ConsumerServerConfig {
    pub bind_address: String,
    pub audio_capture_config: AudioCaptureConfig,
    pub wakeword_models: Vec<String>,
    pub detection_threshold: f32,
    pub vad_config: VadConfig,
    /// `host:port` of the LED controller's HTTP API. The detection thread POSTs
    /// a `ww_detected` event here the instant a wake word fires, before any
    /// media-pause work, for the lowest-latency ring feedback.
    pub led_endpoint: String,
    /// `host:port` of the spotify-control service. The detection thread POSTs a
    /// pause request here on wake word.
    pub spotify_endpoint: String,
}

impl Default for ConsumerServerConfig {
    fn default() -> Self {
        Self {
            bind_address: "127.0.0.1:8080".to_string(),
            audio_capture_config: AudioCaptureConfig::default(),
            wakeword_models: vec!["hey_mycroft".to_string()],
            detection_threshold: 0.5,
            vad_config: VadConfig::default(),
            led_endpoint: "127.0.0.1:3000".to_string(),
            spotify_endpoint: "127.0.0.1:3001".to_string(),
        }
    }
}

/// Consumer server that provides audio stream + events to a single consumer
pub struct ConsumerServer {
    config: ConsumerServerConfig,
    should_stop: Arc<AtomicBool>,
    consumer_connected: Arc<AtomicBool>,
    audio_capture: Arc<Mutex<Option<AudioCapture>>>,
    wakeword_model: Arc<Mutex<Option<WakewordModel>>>,
    vad_processor: Arc<Mutex<Option<VadProcessor>>>,
    spotify_controller: SpotifyController,
    mpv_controller: MpvController,
    barge_in_tx: Option<Sender<()>>,
}

impl ConsumerServer {
    pub fn new(config: ConsumerServerConfig) -> Self {
        let spotify_controller = SpotifyController::new(config.spotify_endpoint.clone());
        Self {
            config,
            should_stop: Arc::new(AtomicBool::new(false)),
            consumer_connected: Arc::new(AtomicBool::new(false)),
            audio_capture: Arc::new(Mutex::new(None)),
            wakeword_model: Arc::new(Mutex::new(None)),
            vad_processor: Arc::new(Mutex::new(None)),
            spotify_controller,
            mpv_controller: MpvController::new(),
            barge_in_tx: None,
        }
    }

    /// Set the barge-in sender (call before run())
    pub fn set_barge_in_sender(&mut self, tx: Sender<()>) {
        self.barge_in_tx = Some(tx);
    }

    /// Start the detection thread and return the receiver for audio-detection pairs
    fn start_detection_thread(&self) -> Result<Receiver<AudioDetectionPair>, ConsumerServerError> {
        let capacity = 20;
        let (sender, receiver) = crossbeam::channel::bounded(capacity);

        // Clone resources for detection thread
        let should_stop = Arc::clone(&self.should_stop);
        let consumer_connected = Arc::clone(&self.consumer_connected);
        let audio_capture = Arc::clone(&self.audio_capture);
        let wakeword_model = Arc::clone(&self.wakeword_model);
        let vad_processor = Arc::clone(&self.vad_processor);
        let config = self.config.clone();
        let spotify_controller = self.spotify_controller.clone();
        let mpv_controller = self.mpv_controller.clone();
        let barge_in_tx = self.barge_in_tx.clone();

        // Start detection thread
        thread::spawn(move || {
            let result = Self::detection_thread(
                should_stop,
                consumer_connected,
                audio_capture,
                wakeword_model,
                vad_processor,
                config,
                sender,
                spotify_controller,
                mpv_controller,
                barge_in_tx,
            );

            if let Err(e) = result {
                log::error!("❌ Detection thread failed: {}", e);
            }
        });

        Ok(receiver)
    }

    /// Start the consumer server (blocking)
    pub fn run(&self) -> Result<(), ConsumerServerError> {
        log::info!(
            "🎯 Starting Consumer TCP server on {}",
            self.config.bind_address
        );

        // Start detection thread first (runs independently)
        let detection_receiver = self.start_detection_thread()?;
        log::info!("✅ Detection thread started");

        let listener = TcpListener::bind(&self.config.bind_address)?;
        listener.set_nonblocking(true)?;

        log::info!(
            "🎯 Consumer server listening on {}",
            self.config.bind_address
        );

        // Note: Signal handling is done in main.rs via stop() method

        while !self.should_stop.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((stream, addr)) => {
                    log::info!("🎯 Consumer connection attempt from {}", addr);

                    // Check if we already have a consumer
                    if self.consumer_connected.load(Ordering::SeqCst) {
                        log::warn!("⚠️  Rejecting consumer from {}: already connected", addr);
                        self.reject_consumer(stream, "Consumer already connected".to_string());
                        continue;
                    }

                    // Handle the consumer connection
                    self.handle_consumer(stream, addr.to_string(), &detection_receiver);
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    // No connection available, sleep and continue
                    thread::sleep(Duration::from_millis(100));
                }
                Err(e) => {
                    log::error!("❌ Error accepting consumer connection: {}", e);
                    thread::sleep(Duration::from_millis(1000));
                }
            }
        }

        log::info!("🛑 Consumer server shutting down");
        Ok(())
    }

    /// Reject a consumer connection with an error message
    fn reject_consumer(&self, stream: TcpStream, error_message: String) {
        let mut connection = ConsumerConnection::new(stream);
        let error_msg = ConsumerMessage::Error {
            message: error_message,
        };

        if let Err(e) = connection.write_message(&error_msg) {
            log::error!(
                "❌ Failed to send error message to rejected consumer: {}",
                e
            );
        }
        // Connection will be dropped when this function returns
    }

    /// Detection thread that processes audio and generates detection events
    fn detection_thread(
        should_stop: Arc<AtomicBool>,
        consumer_connected: Arc<AtomicBool>,
        audio_capture: Arc<Mutex<Option<AudioCapture>>>,
        wakeword_model: Arc<Mutex<Option<WakewordModel>>>,
        vad_processor: Arc<Mutex<Option<VadProcessor>>>,
        config: ConsumerServerConfig,
        sender: Sender<AudioDetectionPair>,
        spotify_controller: SpotifyController,
        mpv_controller: MpvController,
        barge_in_tx: Option<Sender<()>>,
    ) -> Result<(), ConsumerServerError> {
        // Initialize audio capture for streaming
        {
            let mut capture_guard = audio_capture.lock().unwrap();
            if capture_guard.is_none() {
                log::info!(
                    "🎤 Initializing audio capture for streaming (channel {})",
                    config.audio_capture_config.channel
                );
                match AudioCapture::new(config.audio_capture_config.clone()) {
                    Ok(capture) => {
                        *capture_guard = Some(capture);
                    }
                    Err(e) => {
                        return Err(ConsumerServerError::Audio(e.to_string()));
                    }
                }
            }
        }

        // Initialize wakeword model
        {
            let mut model_guard = wakeword_model.lock().unwrap();
            if model_guard.is_none() {
                log::info!("🎯 Initializing wakeword model for detection");
                match WakewordModel::new(config.wakeword_models.clone(), vec![]) {
                    Ok(model) => {
                        *model_guard = Some(model);
                        log::info!(
                            "✅ Wakeword model loaded with {} models",
                            config.wakeword_models.len()
                        );
                    }
                    Err(e) => {
                        return Err(ConsumerServerError::Audio(format!(
                            "Wakeword model error: {}",
                            e
                        )));
                    }
                }
            }
        }

        // Initialize VAD processor
        {
            let mut vad_guard = vad_processor.lock().unwrap();
            if vad_guard.is_none() {
                log::info!("🎤 Initializing VAD processor for detection");
                match VadProcessor::new(config.vad_config.clone()) {
                    Ok(vad) => {
                        *vad_guard = Some(vad);
                        log::info!("✅ VAD processor initialized");
                    }
                    Err(e) => {
                        return Err(ConsumerServerError::Audio(format!("VAD error: {}", e)));
                    }
                }
            }
        }

        #[cfg(target_os = "linux")]
        unsafe {
            libc::setpriority(libc::PRIO_PROCESS, 0, 10);
            log::info!("Detection thread nice value set to 10 (lower priority)");

            let mut cpuset: libc::cpu_set_t = std::mem::zeroed();
            libc::CPU_SET(2, &mut cpuset);
            libc::CPU_SET(3, &mut cpuset);
            let ret = libc::sched_setaffinity(
                0,
                std::mem::size_of::<libc::cpu_set_t>(),
                &cpuset,
            );
            if ret == 0 {
                log::info!("Detection thread pinned to cores 2-3");
            } else {
                log::warn!(
                    "Failed to pin detection thread to cores 2-3: {}",
                    std::io::Error::last_os_error()
                );
            }
        }

        log::info!("🎵 Starting audio detection processing");

        let mut last_wakeword_time: Option<Instant> = None;
        let mut detection_attempts = 0u64;
        let mut audio_chunks_processed = 0u64;
        let start_time = Instant::now();
        let mut dropped_pairs = 0u64;

        const WAKEWORD_DEBOUNCE_MS: u64 = 3000;

        // Near-miss instrumentation: when WW_NEARMISS_LOG=1, track the peak wake
        // confidence across each VAD speech segment and log it when the segment
        // ends without firing. This surfaces sub-threshold "Hey Mycroft" misses
        // that are otherwise invisible (only >= threshold detections are logged).
        let nearmiss_log = std::env::var("WW_NEARMISS_LOG")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let mut last_ww_max_conf = 0.0f32;
        let mut segment_peak_conf = 0.0f32;
        let mut in_speech_segment = false;
        let mut segment_fired = false;
        let mut silence_run = 0u32;
        // End a speech segment after this many consecutive non-speech chunks, so
        // brief VAD flicker mid-utterance doesn't split one phrase into several.
        const SEGMENT_SILENCE_CHUNKS: u32 = 25;

        while !should_stop.load(Ordering::SeqCst) {
            let audio = {
                let capture_guard = audio_capture.lock().unwrap();
                capture_guard.as_ref().and_then(|c| c.try_next_chunk())
            };

            if let Some(ref chunk_data) = audio {
                let samples: Vec<i16> = chunk_data
                    .chunks_exact(2)
                    .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
                    .collect();

                if !samples.is_empty() {
                    audio_chunks_processed += 1;

                    // Feed each chunk directly to the stateful model exactly once.
                    // The model's preprocessor maintains internal mel/embedding buffers
                    // across calls, so re-feeding overlapping windows would corrupt state.
                    let wakeword_event = {
                        detection_attempts += 1;

                        if detection_attempts % 100 == 0 {
                            let elapsed = start_time.elapsed();
                            log::debug!(
                                "📊 [Detection] Performance stats: {} detections in {:.1}s, {} audio chunks, rate={:.1} detections/min, dropped={}",
                                detection_attempts,
                                elapsed.as_secs_f64(),
                                audio_chunks_processed,
                                (detection_attempts as f64) / elapsed.as_secs_f64() * 60.0,
                                dropped_pairs
                            );
                        }

                        let (event_opt, ww_max_conf) =
                            Self::process_wakeword_detection_standalone(
                                &wakeword_model,
                                &samples,
                                config.detection_threshold,
                                &last_wakeword_time,
                                WAKEWORD_DEBOUNCE_MS,
                                &spotify_controller,
                                &mpv_controller,
                                &barge_in_tx,
                                &config.led_endpoint,
                            )?;
                        last_ww_max_conf = ww_max_conf;
                        match event_opt {
                            Some(detection) => {
                                last_wakeword_time = Some(detection.1);
                                Some(detection.0)
                            }
                            None => None,
                        }
                    };

                    let speech_detected = {
                        let mut vad_guard = vad_processor.lock().unwrap();
                        if let Some(ref mut vad) = vad_guard.as_mut() {
                            match vad.analyze_chunk(chunk_data) {
                                Ok(has_speech) => has_speech,
                                Err(e) => {
                                    log::warn!("⚠️ VAD processing error: {}", e);
                                    false
                                }
                            }
                        } else {
                            false
                        }
                    };

                    if nearmiss_log {
                        if speech_detected {
                            in_speech_segment = true;
                            silence_run = 0;
                            if last_ww_max_conf > segment_peak_conf {
                                segment_peak_conf = last_ww_max_conf;
                            }
                            if wakeword_event.is_some() {
                                segment_fired = true;
                            }
                        } else if in_speech_segment {
                            silence_run += 1;
                            if silence_run >= SEGMENT_SILENCE_CHUNKS {
                                if !segment_fired {
                                    log::info!(
                                        "🔎 [WW-NEARMISS] speech segment ended without firing: peak_confidence={:.3} (threshold={:.2})",
                                        segment_peak_conf,
                                        config.detection_threshold
                                    );
                                }
                                in_speech_segment = false;
                                segment_peak_conf = 0.0;
                                segment_fired = false;
                                silence_run = 0;
                            }
                        }
                    }

                    let pair = AudioDetectionPair {
                        audio_data: chunk_data.clone(),
                        speech_detected,
                        wakeword_event,
                        timestamp: ConsumerMessage::current_timestamp(),
                    };

                    if consumer_connected.load(Ordering::SeqCst) {
                        match sender.try_send(pair) {
                            Ok(()) => {}
                            Err(crossbeam::channel::TrySendError::Full(_)) => {
                                dropped_pairs += 1;
                                if dropped_pairs % 10 == 0 {
                                    log::warn!("⚠️ [Detection] Backpressure: dropped {} audio pairs, consumer lagging", dropped_pairs);
                                }
                            }
                            Err(crossbeam::channel::TrySendError::Disconnected(_)) => {
                                log::debug!(
                                    "🔌 Detection thread: consumer disconnected during send"
                                );
                            }
                        }
                    }
                }
            } else {
                thread::sleep(Duration::from_millis(10));
            }
        }

        log::info!("🛑 Detection thread ended");
        Ok(())
    }

    /// Process wakeword detection without consumer connection (standalone)
    /// Returns `(Some((WakewordEvent, timestamp)), peak_confidence)` if a wake
    /// word fired, else `(None, peak_confidence)`. The peak confidence across
    /// models is always returned so callers can log sub-threshold near-misses.
    fn process_wakeword_detection_standalone(
        wakeword_model: &Arc<Mutex<Option<WakewordModel>>>,
        detection_samples: &[i16],
        threshold: f32,
        last_wakeword_time: &Option<Instant>,
        debounce_ms: u64,
        spotify_controller: &SpotifyController,
        mpv_controller: &MpvController,
        barge_in_tx: &Option<Sender<()>>,
        led_endpoint: &str,
    ) -> Result<(Option<(WakewordEvent, Instant)>, f32), ConsumerServerError> {
        let mut max_conf = 0.0f32;
        if let Some(ref mut model) = wakeword_model.lock().unwrap().as_mut() {
            // Time the TFLite inference so we can tell, on-device, how much of
            // the end-to-end latency is the model itself vs. the pause work.
            let predict_start = Instant::now();
            match model.predict(detection_samples, None, 1.0) {
                Ok(predictions) => {
                    let predict_ms = predict_start.elapsed().as_secs_f64() * 1000.0;
                    // Check predictions against threshold
                    for (model_name, confidence) in predictions {
                        if confidence > max_conf {
                            max_conf = confidence;
                        }
                        if confidence >= threshold {
                            // Check debouncing - don't send wake word if we sent one recently
                            let now = Instant::now();
                            let should_debounce = if let Some(last_time) = last_wakeword_time {
                                now.duration_since(*last_time).as_millis() < debounce_ms as u128
                            } else {
                                false
                            };

                            if should_debounce {
                                log::debug!(
                                    "🔇 [Detection] Wake word '{}' debounced (confidence {:.6}) - last detection was {:.1}ms ago",
                                    model_name,
                                    confidence,
                                    last_wakeword_time.unwrap().elapsed().as_millis()
                                );
                                continue;
                            }

                            log::info!(
                                "🎯 [Detection] WAKEWORD DETECTED: '{}' with confidence {:.6} (tflite inference {:.1}ms)",
                                model_name,
                                confidence,
                                predict_ms
                            );

                            // --- Immediate feedback, BEFORE any blocking work ---
                            // Light the ring instantly via the LED controller's
                            // HTTP API (fire-and-forget, never blocks). This is
                            // the lowest-latency feedback path: it does not wait
                            // for the agent's TCP + STT round-trip.
                            crate::led_notify::notify_ww_detected(led_endpoint);

                            // Send barge-in signal to producer (automatic server-side barge-in)
                            // Use try_send - non-blocking, stale signals will be drained by producer
                            if let Some(ref barge_in) = barge_in_tx {
                                match barge_in.try_send(()) {
                                    Ok(()) => {
                                        log::info!("🔥 Sent barge-in signal to producer (automatic interruption)");
                                    }
                                    Err(e) => {
                                        log::debug!("Barge-in signal not sent (producer may not be playing): {}", e);
                                    }
                                }
                            }

                            // --- Pause Spotify + mpv concurrently ---
                            // Each pause runs on its own thread so total latency
                            // is max(spotify, mpv) instead of the sum. Both
                            // controllers are cheap to clone (the Spotify one
                            // caches its D-Bus connection internally).
                            let pause_start = Instant::now();
                            let spotify = spotify_controller.clone();
                            let spotify_handle = thread::spawn(move || {
                                let t = Instant::now();
                                let paused = spotify.pause_for_wakeword();
                                (paused, t.elapsed())
                            });
                            let mpv = mpv_controller.clone();
                            let mpv_handle = thread::spawn(move || {
                                let t = Instant::now();
                                let paused = mpv.pause_for_wakeword();
                                (paused, t.elapsed())
                            });

                            let (spotify_was_paused, spotify_dur) =
                                spotify_handle.join().unwrap_or((false, Duration::ZERO));
                            let (mpv_was_paused, mpv_dur) =
                                mpv_handle.join().unwrap_or((false, Duration::ZERO));

                            log::info!(
                                "⏱️ [Detection] media pause done in {:.1}ms (spotify {:.1}ms was_paused={}, mpv {:.1}ms was_paused={})",
                                pause_start.elapsed().as_secs_f64() * 1000.0,
                                spotify_dur.as_secs_f64() * 1000.0,
                                spotify_was_paused,
                                mpv_dur.as_secs_f64() * 1000.0,
                                mpv_was_paused
                            );

                            // --- Confirmation beep ---
                            // Only beep when nothing was actually paused: if
                            // media was playing, the pause itself is obvious
                            // feedback, so a beep would just be redundant noise.
                            if !spotify_was_paused && !mpv_was_paused {
                                crate::beep::play_confirmation();
                            }

                            let wakeword_event = WakewordEvent {
                                model: model_name,
                                confidence,
                                timestamp: ConsumerMessage::current_timestamp(),
                                spotify_was_paused,
                                mpv_was_paused,
                            };

                            return Ok((Some((wakeword_event, now)), max_conf));
                        }
                    }
                }
                Err(e) => {
                    log::warn!("[Detection] Wakeword detection failed: {}", e);
                }
            }
        }
        Ok((None, max_conf)) // No wake word fired; return peak for near-miss logging
    }

    /// Handle a single consumer connection
    fn handle_consumer(
        &self,
        stream: TcpStream,
        addr: String,
        detection_receiver: &Receiver<AudioDetectionPair>,
    ) {
        // Mark consumer as connected
        self.consumer_connected.store(true, Ordering::SeqCst);

        // Spawn thread to handle this consumer
        let should_stop = Arc::clone(&self.should_stop);
        let consumer_connected = Arc::clone(&self.consumer_connected);

        // Clone the detection receiver for the consumer thread
        let detection_receiver_clone = detection_receiver.clone();

        thread::spawn(move || {
            let result = Self::consumer_thread(
                stream,
                addr.clone(),
                should_stop.clone(),
                consumer_connected.clone(),
                detection_receiver_clone,
            );

            // Always mark consumer as disconnected when thread exits
            consumer_connected.store(false, Ordering::SeqCst);

            match result {
                Ok(()) => {
                    log::info!("✅ Consumer {} disconnected cleanly", addr);
                }
                Err(e) => {
                    log::error!("❌ Consumer {} error: {}", addr, e);
                }
            }

            log::info!(
                "🔌 Consumer {} connection ended, server remains available for new connections",
                addr
            );
        });
    }

    /// Consumer thread that handles the consumer connection and streams audio + events
    fn consumer_thread(
        stream: TcpStream,
        addr: String,
        should_stop: Arc<AtomicBool>,
        _consumer_connected: Arc<AtomicBool>,
        detection_receiver: Receiver<AudioDetectionPair>,
    ) -> Result<(), ConsumerServerError> {
        let mut connection = ConsumerConnection::new(stream);

        // No subscription needed - client can start receiving immediately
        log::info!("✅ Consumer {} connected successfully", addr);

        log::info!(
            "🎵 Starting channel-based audio streaming for consumer {}",
            addr
        );

        let mut received_pairs = 0u64;
        let mut sent_audio = 0u64;
        let mut sent_wakewords = 0u64;
        let dropped_by_consumer = 0u64;
        let start_time = Instant::now();

        while !should_stop.load(Ordering::SeqCst) {
            // Receive audio-detection pairs from detection thread
            match detection_receiver.recv_timeout(Duration::from_millis(100)) {
                Ok(pair) => {
                    received_pairs += 1;

                    // Send audio chunk to consumer
                    let audio_msg = ConsumerMessage::Audio {
                        data: pair.audio_data,
                        speech_detected: pair.speech_detected,
                        timestamp: pair.timestamp,
                    };

                    match connection.write_message(&audio_msg) {
                        Ok(()) => {
                            sent_audio += 1;
                        }
                        Err(e) => {
                            log::error!("❌ Failed to send audio to consumer {}: {}", addr, e);
                            break;
                        }
                    }

                    // Send wakeword event if present
                    if let Some(wakeword_event) = pair.wakeword_event {
                        let wakeword_msg = ConsumerMessage::WakewordDetected {
                            model: wakeword_event.model.clone(),
                            timestamp: wakeword_event.timestamp,
                            spotify_was_paused: wakeword_event.spotify_was_paused,
                            mpv_was_paused: wakeword_event.mpv_was_paused,
                        };

                        match connection.write_message(&wakeword_msg) {
                            Ok(()) => {
                                sent_wakewords += 1;
                                log::info!(
                                    "🎯 [{}] Sent wakeword to consumer: {} (confidence: {:.6})",
                                    addr,
                                    wakeword_event.model,
                                    wakeword_event.confidence
                                );
                            }
                            Err(e) => {
                                log::error!(
                                    "❌ Failed to send wakeword to consumer {}: {}",
                                    addr,
                                    e
                                );
                                break;
                            }
                        }
                    }

                    // Log consumer performance stats every 100 audio chunks
                    if sent_audio % 100 == 0 {
                        let elapsed = start_time.elapsed();
                        log::debug!(
                            "📊 [{}] Consumer stats: received={} sent_audio={} sent_wakewords={} dropped={} in {:.1}s",
                            addr,
                            received_pairs,
                            sent_audio,
                            sent_wakewords,
                            dropped_by_consumer,
                            elapsed.as_secs_f64()
                        );
                    }
                }
                Err(crossbeam::channel::RecvTimeoutError::Timeout) => {
                    // No data available, continue loop to check should_stop
                    continue;
                }
                Err(crossbeam::channel::RecvTimeoutError::Disconnected) => {
                    log::warn!(
                        "🔌 [{}] Detection thread disconnected, ending consumer",
                        addr
                    );
                    break;
                }
            }
        }

        log::info!("🛑 Consumer {} disconnected", addr);
        Ok(())
    }

    /// Stop the server
    pub fn stop(&self) {
        self.should_stop.store(true, Ordering::SeqCst);
    }
}
