use crate::audio_source::{AudioCapture, AudioCaptureConfig};
use crate::protocol::{ConsumerConnection, ConsumerMessage, ProtocolError};
use crate::wakeword_model::Model as WakewordModel;
use crate::wakeword_vad::{AudioEvent, VadConfig, VadProcessor};
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

/// Configuration for the consumer server
#[derive(Clone)]
pub struct ConsumerServerConfig {
    pub bind_address: String,
    pub audio_capture_config: AudioCaptureConfig,
    pub wakeword_models: Vec<String>, // Models to load (e.g., ["hey_mycroft"])
    pub detection_threshold: f32,     // Wakeword detection threshold
    pub vad_config: VadConfig,        // VAD configuration
}

impl Default for ConsumerServerConfig {
    fn default() -> Self {
        Self {
            bind_address: "127.0.0.1:8080".to_string(),
            audio_capture_config: AudioCaptureConfig::default(),
            wakeword_models: vec!["hey_mycroft".to_string()], // Default model
            detection_threshold: 0.5,                         // Default threshold
            vad_config: VadConfig::default(),                 // Default VAD config
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
}

impl ConsumerServer {
    pub fn new(config: ConsumerServerConfig) -> Self {
        Self {
            config,
            should_stop: Arc::new(AtomicBool::new(false)),
            consumer_connected: Arc::new(AtomicBool::new(false)),
            audio_capture: Arc::new(Mutex::new(None)),
            wakeword_model: Arc::new(Mutex::new(None)),
            vad_processor: Arc::new(Mutex::new(None)),
        }
    }

    /// Start the consumer server (blocking)
    pub fn run(&self) -> Result<(), ConsumerServerError> {
        log::info!(
            "🎯 Starting Consumer TCP server on {}",
            self.config.bind_address
        );

        let listener = TcpListener::bind(&self.config.bind_address)?;
        listener.set_nonblocking(true)?;

        log::info!(
            "🎯 Consumer server listening on {}",
            self.config.bind_address
        );

        // Signal handling
        let should_stop = Arc::clone(&self.should_stop);
        ctrlc::set_handler(move || {
            log::info!("🛑 Consumer server received shutdown signal");
            should_stop.store(true, Ordering::SeqCst);
        })
        .ok();

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
                    self.handle_consumer(stream, addr.to_string());
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

    /// Handle a single consumer connection
    fn handle_consumer(&self, stream: TcpStream, addr: String) {
        // Mark consumer as connected
        self.consumer_connected.store(true, Ordering::SeqCst);

        // Spawn thread to handle this consumer
        let should_stop = Arc::clone(&self.should_stop);
        let consumer_connected = Arc::clone(&self.consumer_connected);
        let audio_capture = Arc::clone(&self.audio_capture);
        let wakeword_model = Arc::clone(&self.wakeword_model);
        let vad_processor = Arc::clone(&self.vad_processor);
        let config = self.config.clone();

        thread::spawn(move || {
            let result = Self::consumer_thread(
                stream,
                addr.clone(),
                should_stop,
                consumer_connected.clone(),
                audio_capture,
                wakeword_model,
                vad_processor,
                config,
            );

            // Always mark consumer as disconnected when thread exits
            consumer_connected.store(false, Ordering::SeqCst);

            match result {
                Ok(()) => log::info!("✅ Consumer {} disconnected cleanly", addr),
                Err(e) => log::error!("❌ Consumer {} error: {}", addr, e),
            }
        });
    }

    /// Consumer thread that handles the consumer connection and streams audio + events
    fn consumer_thread(
        stream: TcpStream,
        addr: String,
        should_stop: Arc<AtomicBool>,
        _consumer_connected: Arc<AtomicBool>,
        audio_capture: Arc<Mutex<Option<AudioCapture>>>,
        wakeword_model: Arc<Mutex<Option<WakewordModel>>>,
        vad_processor: Arc<Mutex<Option<VadProcessor>>>,
        config: ConsumerServerConfig,
    ) -> Result<(), ConsumerServerError> {
        let mut connection = ConsumerConnection::new(stream);

        // Read the subscribe message
        let subscribe_msg = connection.read_message()?;
        let client_id = match subscribe_msg {
            ConsumerMessage::Subscribe { id } => {
                log::info!("🎯 Consumer {} subscribed with ID: {}", addr, id);
                id
            }
            _ => {
                let error_msg = ConsumerMessage::Error {
                    message: "Expected Subscribe message".to_string(),
                };
                connection.write_message(&error_msg)?;
                return Err(ConsumerServerError::Protocol(
                    ProtocolError::InvalidMessageType(0),
                ));
            }
        };

        // Send Connected confirmation
        connection.write_message(&ConsumerMessage::Connected)?;
        log::info!(
            "✅ Consumer {} ({}) connected successfully",
            addr,
            client_id
        );

        // Initialize audio capture if not already running
        {
            let mut capture_guard = audio_capture.lock().unwrap();
            if capture_guard.is_none() {
                log::info!("🎤 Initializing audio capture for consumer");
                match AudioCapture::new(config.audio_capture_config.clone()) {
                    Ok(capture) => {
                        *capture_guard = Some(capture);
                    }
                    Err(e) => {
                        let error_msg = ConsumerMessage::Error {
                            message: format!("Failed to initialize audio capture: {}", e),
                        };
                        connection.write_message(&error_msg)?;
                        return Err(ConsumerServerError::Audio(e.to_string()));
                    }
                }
            }
        }

        // Initialize wakeword model if not already running
        {
            let mut model_guard = wakeword_model.lock().unwrap();
            if model_guard.is_none() {
                log::info!("🎯 Initializing wakeword model for consumer");
                match WakewordModel::new(config.wakeword_models.clone(), vec![]) {
                    Ok(model) => {
                        *model_guard = Some(model);
                        log::info!(
                            "✅ Wakeword model loaded with {} models",
                            config.wakeword_models.len()
                        );
                    }
                    Err(e) => {
                        let error_msg = ConsumerMessage::Error {
                            message: format!("Failed to initialize wakeword model: {}", e),
                        };
                        connection.write_message(&error_msg)?;
                        return Err(ConsumerServerError::Audio(format!(
                            "Wakeword model error: {}",
                            e
                        )));
                    }
                }
            }
        }

        // Initialize VAD processor if not already running
        {
            let mut vad_guard = vad_processor.lock().unwrap();
            if vad_guard.is_none() {
                log::info!("🎤 Initializing VAD processor for consumer");
                match VadProcessor::new(config.vad_config.clone()) {
                    Ok(vad) => {
                        *vad_guard = Some(vad);
                        log::info!("✅ VAD processor initialized");
                    }
                    Err(e) => {
                        let error_msg = ConsumerMessage::Error {
                            message: format!("Failed to initialize VAD processor: {}", e),
                        };
                        connection.write_message(&error_msg)?;
                        return Err(ConsumerServerError::Audio(format!("VAD error: {}", e)));
                    }
                }
            }
        }

        // Stream audio chunks to consumer with wakeword detection and VAD processing
        log::info!(
            "🎵 Starting audio stream with wakeword detection for consumer {}",
            client_id
        );

        // Audio processing buffers (matching original wakeword logic)
        let mut audio_buffer = VecDeque::new();
        let mut last_detection_time = Instant::now();
        const DETECTION_WINDOW_SAMPLES: usize = 5120; // 320ms at 16kHz
        const DETECTION_INTERVAL_MS: u64 = 160; // Run detection every 160ms
        const MAX_BUFFER_SAMPLES: usize = 16000; // 1 second at 16kHz

        while !should_stop.load(Ordering::SeqCst) {
            // Get next audio chunk (blocking with timeout)
            if let Some(audio_data) = {
                let capture_guard = audio_capture.lock().unwrap();
                capture_guard.as_ref().and_then(|c| c.try_next_chunk())
            } {
                // Convert audio data to i16 samples for processing
                let samples: Vec<i16> = audio_data
                    .chunks_exact(2)
                    .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
                    .collect();

                if !samples.is_empty() {
                    // Add samples to processing buffer FIRST
                    audio_buffer.extend(samples.iter());

                    // Run wakeword detection BEFORE forwarding audio
                    let has_enough_samples = audio_buffer.len() >= DETECTION_WINDOW_SAMPLES;
                    let enough_time_passed = last_detection_time.elapsed()
                        >= Duration::from_millis(DETECTION_INTERVAL_MS);

                    if has_enough_samples && enough_time_passed {
                        // Run wakeword detection on recent window
                        let detection_samples: Vec<i16> = audio_buffer
                            .iter()
                            .rev()
                            .take(DETECTION_WINDOW_SAMPLES)
                            .rev()
                            .copied()
                            .collect();

                        Self::process_wakeword_detection(
                            &wakeword_model,
                            &detection_samples,
                            config.detection_threshold,
                            &mut connection,
                            &client_id,
                        )?;
                        last_detection_time = Instant::now();
                    }

                    // Keep buffer from growing too large
                    while audio_buffer.len() > MAX_BUFFER_SAMPLES {
                        audio_buffer.pop_front();
                    }
                }

                // Process audio through VAD and get speech detection result
                let speech_detected =
                    Self::process_vad_for_chunk(&vad_processor, &audio_data, &client_id)?;

                // Forward the audio chunk to consumer with VAD result
                // (wakeword detection has already run and sent any detection messages)
                let audio_msg = ConsumerMessage::Audio {
                    data: audio_data,
                    speech_detected,
                };
                if let Err(e) = connection.write_message(&audio_msg) {
                    log::warn!("❌ Failed to send audio to consumer {}: {}", client_id, e);
                    break;
                }
            } else {
                // No audio available, sleep briefly
                thread::sleep(Duration::from_millis(10));
            }
        }

        log::info!("🛑 Audio stream ended for consumer {}", client_id);
        Ok(())
    }

    /// Stop the server
    pub fn stop(&self) {
        self.should_stop.store(true, Ordering::SeqCst);
    }

    /// Process audio through VAD and return speech detection result for this chunk
    fn process_vad_for_chunk(
        vad_processor: &Arc<Mutex<Option<VadProcessor>>>,
        audio_data: &[u8],
        client_id: &str,
    ) -> Result<bool, ConsumerServerError> {
        // Convert audio data to f32 samples for VAD processing
        let f32_samples: Vec<f32> = audio_data
            .chunks_exact(2)
            .map(|chunk| {
                let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
                sample as f32 / 32768.0 // Convert to [-1.0, 1.0] range
            })
            .collect();

        let mut chunk_has_speech = false;

        // Process in 512-sample chunks for VAD
        for chunk_512 in f32_samples.chunks(512) {
            if chunk_512.len() == 512 {
                let mut samples_array = [0.0f32; 512];
                samples_array.copy_from_slice(chunk_512);

                // Process through VAD
                if let Some(ref mut vad) = vad_processor.lock().unwrap().as_mut() {
                    match vad.process_chunk(&samples_array) {
                        Ok(AudioEvent::StartedAudio) => {
                            log::info!("🗣️ [{}] Speech started", client_id);
                            chunk_has_speech = true;
                        }
                        Ok(AudioEvent::StoppedAudio) => {
                            log::info!("🔇 [{}] Speech stopped", client_id);
                            // Note: We still consider this chunk as having speech since it detected the end
                            chunk_has_speech = true;
                        }
                        Ok(AudioEvent::Audio) => {
                            // Check current VAD state to determine if we're in speech or silence
                            // For now, we'll assume this means ongoing speech if we're in the speech state
                            if vad.current_state() == "speech" {
                                chunk_has_speech = true;
                            }
                        }
                        Err(e) => {
                            log::warn!("⚠️ [{}] VAD processing error: {}", client_id, e);
                        }
                    }
                }
            }
        }

        Ok(chunk_has_speech)
    }

    /// Process audio through wakeword detection and emit wakeword events
    fn process_wakeword_detection(
        wakeword_model: &Arc<Mutex<Option<WakewordModel>>>,
        detection_samples: &[i16],
        threshold: f32,
        connection: &mut ConsumerConnection<TcpStream>,
        client_id: &str,
    ) -> Result<(), ConsumerServerError> {
        if let Some(ref mut model) = wakeword_model.lock().unwrap().as_mut() {
            match model.predict(detection_samples, None, 1.0) {
                Ok(predictions) => {
                    // Check predictions against threshold
                    for (model_name, confidence) in predictions {
                        if confidence >= threshold {
                            log::info!(
                                "🎯 [{}] WAKEWORD DETECTED: '{}' with confidence {:.6}",
                                client_id,
                                model_name,
                                confidence
                            );

                            let wakeword_msg =
                                ConsumerMessage::WakewordDetected { model: model_name };
                            if let Err(e) = connection.write_message(&wakeword_msg) {
                                log::warn!(
                                    "❌ Failed to send WakewordDetected to {}: {}",
                                    client_id,
                                    e
                                );
                            }
                        }
                    }
                }
                Err(e) => {
                    log::warn!("[{}] Wakeword detection failed: {}", client_id, e);
                }
            }
        }
        Ok(())
    }
}
