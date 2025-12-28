use crate::audio_source::{AudioCapture, AudioCaptureConfig};
use crate::protocol::{ConsumerConnection, ConsumerMessage, ProtocolError};
use crate::spotify_controller::SpotifyController;
use crate::wakeword_model::Model as WakewordModel;
use crate::wakeword_vad::{VadConfig, VadProcessor};
use crossbeam_channel::{Receiver, Sender};
use std::collections::VecDeque;
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
    pub spotify_was_paused: bool, // Whether Spotify was paused for this wakeword
}

/// Configuration for the consumer server
#[derive(Clone)]
pub struct ConsumerServerConfig {
    pub bind_address: String,
    pub audio_capture_config: AudioCaptureConfig,
    pub wakeword_channel: u32, // Channel for wakeword detection (0-based)
    pub wakeword_models: Vec<String>, // Models to load (e.g., ["hey_mycroft"])
    pub detection_threshold: f32, // Wakeword detection threshold
    pub vad_config: VadConfig, // VAD configuration
    pub spotify_player: Option<String>, // Optional Spotify player name for playerctl
}

impl Default for ConsumerServerConfig {
    fn default() -> Self {
        Self {
            bind_address: "127.0.0.1:8080".to_string(),
            audio_capture_config: AudioCaptureConfig::default(),
            wakeword_channel: 0, // Default to channel 0
            wakeword_models: vec!["hey_mycroft".to_string()], // Default model
            detection_threshold: 0.5, // Default threshold
            vad_config: VadConfig::default(), // Default VAD config
            spotify_player: None,
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
    spotify_controller: Option<SpotifyController>,
    barge_in_tx: Option<Sender<()>>, // Sends barge-in signal to producer when wakeword detected during playback
}

impl ConsumerServer {
    pub fn new(config: ConsumerServerConfig) -> Self {
        // Create Spotify controller (with optional preferred player)
        // If no player specified, it will auto-detect any available music player
        let spotify_controller = if let Some(player) = config.spotify_player.as_ref() {
            Some(SpotifyController::new_with_player(player.clone()))
        } else {
            Some(SpotifyController::new()) // Auto-detect mode
        };

        Self {
            config,
            should_stop: Arc::new(AtomicBool::new(false)),
            consumer_connected: Arc::new(AtomicBool::new(false)),
            audio_capture: Arc::new(Mutex::new(None)),
            wakeword_model: Arc::new(Mutex::new(None)),
            vad_processor: Arc::new(Mutex::new(None)),
            spotify_controller,
            barge_in_tx: None,
        }
    }

    /// Set the barge-in sender (call before run())
    pub fn set_barge_in_sender(&mut self, tx: Sender<()>) {
        self.barge_in_tx = Some(tx);
    }

    /// Start the detection thread and return the receiver for audio-detection pairs
    fn start_detection_thread(&self) -> Result<Receiver<AudioDetectionPair>, ConsumerServerError> {
        // Create bounded channel for audio-detection pairs (1-2 seconds of audio buffer)
        let capacity = 100; // ~3 seconds at ~30 chunks/sec
        let (sender, receiver) = crossbeam_channel::bounded(capacity);

        // Clone resources for detection thread
        let should_stop = Arc::clone(&self.should_stop);
        let consumer_connected = Arc::clone(&self.consumer_connected);
        let audio_capture = Arc::clone(&self.audio_capture);
        let wakeword_model = Arc::clone(&self.wakeword_model);
        let vad_processor = Arc::clone(&self.vad_processor);
        let config = self.config.clone();
        let spotify_controller = self.spotify_controller.clone();
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
        spotify_controller: Option<SpotifyController>,
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

        // Initialize separate wakeword audio capture if using different channel
        let wakeword_capture = if config.wakeword_channel != config.audio_capture_config.channel {
            log::info!(
                "🎯 Initializing separate wakeword capture (channel {})",
                config.wakeword_channel
            );
            let wakeword_config = AudioCaptureConfig {
                device_id: config.audio_capture_config.device_id.clone(),
                channel: config.wakeword_channel,
            };
            match AudioCapture::new(wakeword_config) {
                Ok(capture) => Some(capture),
                Err(e) => {
                    log::error!("❌ Failed to initialize wakeword audio capture: {}", e);
                    return Err(ConsumerServerError::Audio(e.to_string()));
                }
            }
        } else {
            log::info!(
                "🎯 Using same channel ({}) for both streaming and wakeword detection",
                config.wakeword_channel
            );
            None
        };

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

        log::info!("🎵 Starting audio detection processing");

        // Audio processing buffers and performance tracking
        let mut audio_buffer = VecDeque::new();
        let mut last_detection_time = Instant::now();
        let mut last_wakeword_time: Option<Instant> = None;
        let mut detection_attempts = 0u64;
        let mut audio_chunks_processed = 0u64;
        let start_time = Instant::now();
        let mut dropped_pairs = 0u64;

        const DETECTION_WINDOW_SAMPLES: usize = 5120; // 320ms at 16kHz
        const DETECTION_INTERVAL_MS: u64 = 160; // Run detection every 160ms
        const MAX_BUFFER_SAMPLES: usize = 16000; // 1 second at 16kHz
        const WAKEWORD_DEBOUNCE_MS: u64 = 3000; // Don't detect same wake word for 3 seconds

        while !should_stop.load(Ordering::SeqCst) {
            // Get next audio chunk from appropriate source
            let (streaming_audio, wakeword_audio) = if let Some(ref ww_capture) = wakeword_capture {
                // Using separate channels - get audio from both
                let streaming = {
                    let capture_guard = audio_capture.lock().unwrap();
                    capture_guard.as_ref().and_then(|c| c.try_next_chunk())
                };
                let wakeword = ww_capture.try_next_chunk();
                (streaming, wakeword)
            } else {
                // Using same channel - use same audio for both
                let audio = {
                    let capture_guard = audio_capture.lock().unwrap();
                    capture_guard.as_ref().and_then(|c| c.try_next_chunk())
                };
                (audio.clone(), audio)
            };

            // Process wakeword audio for detection
            if let Some(wakeword_data) = wakeword_audio {
                // Convert wakeword audio data to i16 samples for processing
                let samples: Vec<i16> = wakeword_data
                    .chunks_exact(2)
                    .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
                    .collect();

                if !samples.is_empty() {
                    audio_buffer.extend(samples.iter());
                    audio_chunks_processed += 1;

                    // Run wakeword detection
                    let has_enough_samples = audio_buffer.len() >= DETECTION_WINDOW_SAMPLES;
                    let enough_time_passed = last_detection_time.elapsed()
                        >= Duration::from_millis(DETECTION_INTERVAL_MS);

                    let mut wakeword_event = None;
                    if has_enough_samples && enough_time_passed {
                        let buffer_len = audio_buffer.len();
                        let start_idx = buffer_len.saturating_sub(DETECTION_WINDOW_SAMPLES);
                        let detection_samples: Vec<i16> =
                            audio_buffer.range(start_idx..).copied().collect();

                        detection_attempts += 1;

                        // Log performance stats every 100 detection attempts
                        if detection_attempts % 100 == 0 {
                            let elapsed = start_time.elapsed();
                            log::info!(
                                "📊 [Detection] Performance stats: {} detections in {:.1}s, {} audio chunks, rate={:.1} detections/min, dropped={}",
                                detection_attempts,
                                elapsed.as_secs_f64(),
                                audio_chunks_processed,
                                (detection_attempts as f64) / elapsed.as_secs_f64() * 60.0,
                                dropped_pairs
                            );
                        }

                        if let Some(detection) = Self::process_wakeword_detection_standalone(
                            &wakeword_model,
                            &detection_samples,
                            config.detection_threshold,
                            &last_wakeword_time,
                            WAKEWORD_DEBOUNCE_MS,
                            &spotify_controller,
                            &barge_in_tx,
                        )? {
                            wakeword_event = Some(detection.0);
                            last_wakeword_time = Some(detection.1);
                        }
                        last_detection_time = Instant::now();
                    }

                    // Keep buffer from growing too large
                    while audio_buffer.len() > MAX_BUFFER_SAMPLES {
                        audio_buffer.pop_front();
                    }

                    // Process streaming audio through VAD and create pair for consumers
                    let pair = if let Some(streaming_data) = streaming_audio {
                        let speech_detected = {
                            let mut vad_guard = vad_processor.lock().unwrap();
                            if let Some(ref mut vad) = vad_guard.as_mut() {
                                match vad.analyze_chunk(&streaming_data) {
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

                        Some(AudioDetectionPair {
                            audio_data: streaming_data,
                            speech_detected,
                            wakeword_event,
                            timestamp: ConsumerMessage::current_timestamp(),
                        })
                    } else {
                        // No streaming audio available, create minimal pair with wakeword event only
                        if wakeword_event.is_some() {
                            Some(AudioDetectionPair {
                                audio_data: vec![], // Empty audio data
                                speech_detected: false,
                                wakeword_event,
                                timestamp: ConsumerMessage::current_timestamp(),
                            })
                        } else {
                            None
                        }
                    };

                    // Send to consumer only if consumer is connected and we have a pair
                    if let Some(audio_pair) = pair {
                        if consumer_connected.load(Ordering::SeqCst) {
                            match sender.try_send(audio_pair) {
                                Ok(()) => {
                                    // Successfully sent
                                }
                                Err(crossbeam_channel::TrySendError::Full(_)) => {
                                    dropped_pairs += 1;
                                    if dropped_pairs % 10 == 0 {
                                        log::warn!("⚠️ [Detection] Backpressure: dropped {} audio pairs, consumer lagging", dropped_pairs);
                                    }
                                }
                                Err(crossbeam_channel::TrySendError::Disconnected(_)) => {
                                    log::debug!(
                                        "🔌 Detection thread: consumer disconnected during send"
                                    );
                                }
                            }
                        }
                    } else {
                        // No consumer connected - detection runs "into the void"
                        // This is exactly what we want for testing isolation
                        if dropped_pairs % 100 == 0 && dropped_pairs > 0 {
                            log::debug!(
                                "🔌 [Detection] Running without consumer (no backpressure)"
                            );
                        }
                    }
                }
            } else {
                // No audio available, sleep briefly
                thread::sleep(Duration::from_millis(10));
            }
        }

        log::info!("🛑 Detection thread ended");
        Ok(())
    }

    /// Process wakeword detection without consumer connection (standalone)
    /// Returns (WakewordEvent, timestamp) if a wake word was detected
    fn process_wakeword_detection_standalone(
        wakeword_model: &Arc<Mutex<Option<WakewordModel>>>,
        detection_samples: &[i16],
        threshold: f32,
        last_wakeword_time: &Option<Instant>,
        debounce_ms: u64,
        spotify_controller: &Option<SpotifyController>,
        barge_in_tx: &Option<Sender<()>>,
    ) -> Result<Option<(WakewordEvent, Instant)>, ConsumerServerError> {
        if let Some(ref mut model) = wakeword_model.lock().unwrap().as_mut() {
            match model.predict(detection_samples, None, 1.0) {
                Ok(predictions) => {
                    // Check predictions against threshold
                    for (model_name, confidence) in predictions {
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
                                "🎯 [Detection] WAKEWORD DETECTED: '{}' with confidence {:.6}",
                                model_name,
                                confidence
                            );

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

                            // Try to pause Spotify if controller is available
                            let spotify_was_paused = if let Some(controller) = spotify_controller {
                                match controller.pause_for_wakeword() {
                                    Ok(was_paused) => was_paused,
                                    Err(e) => {
                                        log::warn!("Failed to pause Spotify: {}", e);
                                        false
                                    }
                                }
                            } else {
                                false
                            };

                            let wakeword_event = WakewordEvent {
                                model: model_name,
                                confidence,
                                timestamp: ConsumerMessage::current_timestamp(),
                                spotify_was_paused,
                            };

                            return Ok(Some((wakeword_event, now)));
                        }
                    }
                }
                Err(e) => {
                    log::warn!("[Detection] Wakeword detection failed: {}", e);
                }
            }
        }
        Ok(None) // No wake word detected
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
                        log::info!(
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
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                    // No data available, continue loop to check should_stop
                    continue;
                }
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
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
