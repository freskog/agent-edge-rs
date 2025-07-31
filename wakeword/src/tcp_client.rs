use crate::{error::OpenWakeWordError, Model, server::WakewordServer, vad::{VadProcessor, VadConfig, AudioEvent}};
use audio_protocol::{AudioChunk, AudioClient};
use log::{debug, error, info, warn};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use wakeword_protocol::{WakewordEvent, protocol::{EosReason, UtteranceSessionStarted, SubscriptionType}};

/// Session state for utterance capture
#[derive(Debug)]
struct UtteranceSession {
    session_id: String,
    start_time: Instant,
    speech_detected: bool,
    chunk_count: u64,
    trigger_model: String,
}

/// Simple TCP client for connecting to audio_api and performing wake word detection
pub struct WakewordClient {
    model: Model,
    client: AudioClient,
    detection_threshold: f32,
    client_id: String,

    // Latency tracking fields
    chunk_receive_times: VecDeque<(u64, Instant)>, // (timestamp_ms, receive_time)

    // Buffer fill tracking
    buffer_fill_start: Option<Instant>,
    last_buffer_size_logged: usize,

    // Debouncing fields
    last_positive_detection: Option<Instant>,
    debounce_duration: Duration,

    // Event broadcasting
    wakeword_server: Option<Arc<WakewordServer>>,

    // NEW: VAD and utterance capture
    vad_processor: VadProcessor,
    current_session: Option<UtteranceSession>,
}

impl WakewordClient {
    /// Create a new TCP client that connects to audio_api
    pub fn new(
        server_address: &str,
        model_names: Vec<String>,
        detection_threshold: f32,
    ) -> Result<Self, OpenWakeWordError> {
        let client_id = format!("wakeword_client_{}", std::process::id());

        info!(
            "🔌 [{}] Connecting to audio server at {}",
            client_id, server_address
        );

        // Connect to TCP server
        let client = AudioClient::connect(server_address).map_err(|e| {
            OpenWakeWordError::IoError(std::io::Error::new(
                std::io::ErrorKind::ConnectionRefused,
                format!("Failed to connect to audio server: {}", e),
            ))
        })?;

        // Initialize the wakeword model
        info!(
            "🧠 [{}] Initializing wake word model with {} models",
            client_id,
            model_names.len()
        );
        let model = Model::new_with_tests(
            model_names,
            vec![], // Empty metadata for now
        )?;

        info!(
            "✅ [{}] TCP client initialized with detection threshold {}",
            client_id, detection_threshold
        );

        Ok(Self {
            model,
            client,
            detection_threshold,
            client_id,
            // Initialize latency tracking
            chunk_receive_times: VecDeque::new(),
            buffer_fill_start: None,
            last_buffer_size_logged: 0,
            // Initialize debouncing (2.5 seconds should be enough to prevent duplicate detections)
            last_positive_detection: None,
            debounce_duration: Duration::from_millis(2500),
            // No event broadcasting by default
            wakeword_server: None,

            // NEW: Initialize VAD and utterance capture
            vad_processor: VadProcessor::new(VadConfig::default()).map_err(|e| {
                OpenWakeWordError::IoError(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("Failed to initialize VAD processor: {}", e),
                ))
            })?,
            current_session: None,
        })
    }

    /// Set the wakeword server for broadcasting events
    pub fn set_wakeword_server(&mut self, server: Arc<WakewordServer>) {
        self.wakeword_server = Some(server);
        info!("[{}] Enabled wakeword event broadcasting", self.client_id);
    }

    /// Start listening for audio and detecting wake words
    pub fn start_detection(&mut self) -> Result<(), OpenWakeWordError> {
        info!("🎯 [{}] Starting wake word detection...", self.client_id);

        // Subscribe to audio stream
        self.client.subscribe_audio().map_err(|e| {
            OpenWakeWordError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to subscribe to audio: {}", e),
            ))
        })?;

        info!(
            "📡 [{}] Subscribed to audio stream, processing chunks...",
            self.client_id
        );

        let mut audio_buffer = Vec::new();
        let mut chunk_count = 0;
        let expected_sample_rate = 16000; // Wake word models expect 16kHz

        // Performance tracking
        let start_time: Instant = Instant::now();
        let mut total_processing_time = Duration::new(0, 0);
        let mut detection_count = 0;

        // Detection frequency optimization - only run every N chunks
        let mut last_detection_time = Instant::now();

        loop {
            match self.client.read_audio_chunk() {
                Ok(Some(chunk)) => {
                    chunk_count += 1;

                    let processing_start = Instant::now();
                    if let Err(e) = self.process_audio_chunk(
                        &chunk,
                        &mut audio_buffer,
                        expected_sample_rate,
                        &mut last_detection_time,
                        &mut detection_count,
                    ) {
                        warn!(
                            "[{}] Failed to process audio chunk {}: {}",
                            self.client_id, chunk_count, e
                        );
                        continue;
                    }

                    let processing_time = processing_start.elapsed();
                    total_processing_time += processing_time;

                }
                Ok(None) => {
                    info!("📡 [{}] Audio stream ended gracefully", self.client_id);
                    break;
                }
                Err(e) => {
                    error!("❌ [{}] Error receiving audio chunk: {}", self.client_id, e);

                    // For systemd managed services, exit immediately on any connection error
                    // so systemd can restart us and reconnect to the audio server
                    error!("💀 [{}] Connection lost, exiting for systemd restart", self.client_id);
                    break;
                }
            }

            // Check for overall timeout
            if start_time.elapsed() > Duration::from_secs(60) && chunk_count == 0 {
                error!("💀 [{}] No audio chunks received in 60 seconds, audio server may not be working properly", self.client_id);
                break;
            }
        }

        if chunk_count > 0 {
            let elapsed = start_time.elapsed();
            let avg_processing = total_processing_time.as_millis() / chunk_count as u128;
            info!(
                "✅ [{}] Final stats: {} chunks in {:?} | avg: {}ms/chunk | {} detections",
                self.client_id, chunk_count, elapsed, avg_processing, detection_count
            );
        } else {
            warn!(
                "⚠️  [{}] No audio chunks were processed - check audio server configuration",
                self.client_id
            );
        }

        Ok(())
    }

    /// Process a single audio chunk and perform detection with comprehensive latency diagnostics
    fn process_audio_chunk(
        &mut self,
        chunk: &AudioChunk,
        audio_buffer: &mut Vec<i16>,
        _expected_sample_rate: u32,
        last_detection_time: &mut Instant,
        detection_count: &mut u64,
    ) -> Result<(), OpenWakeWordError> {
        let chunk_receive_time = Instant::now();

        // Convert audio data to i16 samples
        let conversion_start = Instant::now();
        let samples: Vec<i16> = chunk
            .data
            .chunks_exact(2)
            .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();

        if samples.is_empty() {
            debug!("[{}] Received empty audio chunk, skipping", self.client_id);
            return Ok(());
        }

        // === BUFFER ACCUMULATION DIAGNOSTICS ===
        let buffer_size_before = audio_buffer.len();
        audio_buffer.extend_from_slice(&samples);
        let buffer_size_after = audio_buffer.len();

        // Detection window size (320ms at 16kHz = 5120 samples, 4 chunks)
        const DETECTION_WINDOW_SAMPLES: usize = 5120;

        // Track when we start accumulating buffer for detection
        if buffer_size_before < DETECTION_WINDOW_SAMPLES && self.buffer_fill_start.is_none() {
            self.buffer_fill_start = Some(chunk_receive_time);
        }

        const DETECTION_INTERVAL_MS: u64 = 160; // Run detection every 160ms for real-time processing

        let has_enough_samples = buffer_size_after >= DETECTION_WINDOW_SAMPLES;
        let enough_time_passed =
            last_detection_time.elapsed() >= Duration::from_millis(DETECTION_INTERVAL_MS);
        let should_detect = has_enough_samples && enough_time_passed;

        if should_detect {

            // Take the most recent window for detection
            let detection_samples = audio_buffer
                .iter()
                .skip(audio_buffer.len().saturating_sub(DETECTION_WINDOW_SAMPLES))
                .copied()
                .collect::<Vec<i16>>();

            // === MODEL INFERENCE TIMING ===
            let inference_start = Instant::now();
            match self.model.predict(&detection_samples, None, 1.0) {
                Ok(predictions) => {
                    let inference_time = inference_start.elapsed();

                    self.handle_predictions(predictions)?;

                    // Increment detection count only when detection actually runs
                    *detection_count += 1;
                }
                Err(e) => {
                    warn!("[{}] Wake word detection failed: {}", self.client_id, e);
                }
            }

            // Update timestamp AFTER detection completes to prevent cascading detections
            *last_detection_time = Instant::now();
            self.buffer_fill_start = None; // Reset for next accumulation cycle

            // Keep buffer from growing too large - keep last 1 second
            const MAX_BUFFER_SAMPLES: usize = 16000; // 1 second at 16kHz
            if audio_buffer.len() > MAX_BUFFER_SAMPLES {
                let keep_from = audio_buffer.len() - MAX_BUFFER_SAMPLES;
                let dropped_samples = keep_from;
                let dropped_ms = (dropped_samples as f32 / 16000.0 * 1000.0) as u32;
                
                debug!(
                    "⚠️ [{}] AUDIO BUFFER OVERFLOW: Dropping {} samples ({} ms of audio) - buffer was {} samples",
                    self.client_id, dropped_samples, dropped_ms, audio_buffer.len()
                );
                
                audio_buffer.drain(0..keep_from);
                
                debug!(
                    "🔧 [{}] Audio buffer trimmed to {} samples (1 second retained)",
                    self.client_id, audio_buffer.len()
                );
            }
        }

        Ok(())
    }

    /// Handle wake word detection results
    fn handle_predictions(&mut self, predictions: HashMap<String, f32>) -> Result<(), OpenWakeWordError> {
        let current_time = Instant::now();
        
        // Check if we're in debounce period
        let in_debounce_period = self.last_positive_detection
            .map(|last_detection| current_time.duration_since(last_detection) < self.debounce_duration)
            .unwrap_or(false);

        for (model_name, confidence) in predictions {
            if confidence > self.detection_threshold {
                if in_debounce_period {
                    debug!(
                        "🚫 [{}] WAKE WORD DEBOUNCED: '{}' confidence {:.6} (debouncing for {:.1}s more)",
                        self.client_id, 
                        model_name, 
                        confidence,
                        self.debounce_duration.as_secs_f32() - self.last_positive_detection
                            .map(|last| current_time.duration_since(last).as_secs_f32())
                            .unwrap_or(0.0)
                    );
                } else {
                    info!(
                        "🎯 [{}] WAKE WORD DETECTED: '{}' with confidence {:.6}",
                        self.client_id, model_name, confidence
                    );

                    // Update debounce timer
                    self.last_positive_detection = Some(current_time);

                    // Broadcast wakeword event if server is available
                    info!("🔍 [{}] Checking if wakeword server is available: {}", 
                        self.client_id, self.wakeword_server.is_some());
                    
                    if let Some(ref server) = self.wakeword_server {
                        info!("🔍 [{}] Wakeword server found, checking subscribers first", self.client_id);
                        
                        // Check for utterance subscribers BEFORE broadcasting to avoid deadlock
                        let has_subscribers = server.has_utterance_subscribers();
                        info!("🔍 [{}] Checking utterance subscribers: has_subscribers={}", self.client_id, has_subscribers);
                        
                        // Now broadcast the event
                        let event = WakewordEvent::new(
                            model_name.clone(),
                            confidence,
                            self.client_id.clone(),
                        );
                        server.broadcast_event(event);
                        info!("🔍 [{}] Event broadcasted successfully", self.client_id);
                        
                        // Start utterance capture session if there are subscribers who want it
                        if has_subscribers {
                            info!("🎤 [{}] Starting utterance session after wake word detection", self.client_id);
                            self.start_utterance_capture_session(&model_name)?;
                        } else {
                            info!("⚠️ [{}] No utterance subscribers found, skipping session", self.client_id);
                        }
                    } else {
                        info!("⚠️ [{}] No wakeword server configured, cannot broadcast event", self.client_id);
                    }
                }
            } else {
                // Always log predictions to debug what's happening (increased precision)
                debug!(
                    "🔍 [{}] Detection: '{}' confidence {:.6} (threshold: {:.3})",
                    self.client_id, model_name, confidence, self.detection_threshold
                );
            }
        }
        
        Ok(())
    }

    /// Start an utterance capture session after wake word detection
    fn start_utterance_capture_session(&mut self, trigger_model: &str) -> Result<(), OpenWakeWordError> {
        // Don't start a new session if one is already active
        if self.current_session.is_some() {
            warn!("[{}] Utterance session already active, ignoring new trigger", self.client_id);
            return Ok(());
        }

        let session_id = format!("{}-{}", self.client_id, 
            SystemTime::now().duration_since(UNIX_EPOCH)
                .unwrap_or_default().as_millis());
        
        info!("🎤 [{}] Starting utterance capture session {} (triggered by: {})", 
            self.client_id, session_id, trigger_model);

        // Create and store session state
        self.current_session = Some(UtteranceSession {
            session_id: session_id.clone(),
            start_time: Instant::now(),
            speech_detected: false,
            chunk_count: 0,
            trigger_model: trigger_model.to_string(),
        });

        // Broadcast session started event
        if let Some(ref server) = self.wakeword_server {
            let session_started = UtteranceSessionStarted {
                session_id,
                timestamp: SystemTime::now().duration_since(UNIX_EPOCH)
                    .unwrap_or_default().as_millis() as u64,
                subscription_type: SubscriptionType::WakewordPlusUtterance,
                trigger_model: Some(trigger_model.to_string()),
            };
            server.broadcast_message(crate::server::StreamingMessage::UtteranceSessionStarted(session_started));
        }

        // Start the utterance capture loop
        self.utterance_capture_loop()
    }

    /// Main utterance capture loop - streams audio until end of speech
    fn utterance_capture_loop(&mut self) -> Result<(), OpenWakeWordError> {
        let session = match &self.current_session {
            Some(session) => session,
            None => return Ok(()), // No active session
        };

        let session_id = session.session_id.clone();
        info!("🎵 [{}] Starting utterance capture loop for session {}", self.client_id, session_id);

        // Safety limits to prevent runaway sessions
        const MAX_UTTERANCE_DURATION: Duration = Duration::from_secs(60);
        const PRE_SPEECH_TIMEOUT: Duration = Duration::from_secs(10); // Wait for speech to start

        let session_start = Instant::now();
        
        // Audio streaming loop
        loop {
            // Safety check - absolute maximum duration
            if session_start.elapsed() > MAX_UTTERANCE_DURATION {
                info!("⏰ [{}] Session {} reached maximum duration, ending", self.client_id, session_id);
                self.end_utterance_session(EosReason::Timeout)?;
                break;
            }

            // Read next audio chunk
            match self.client.read_audio_chunk() {
                Ok(Some(audio_chunk)) => {
                    // Process this chunk with VAD and potentially stream it
                    let should_continue = self.process_utterance_chunk(&audio_chunk, session_start, PRE_SPEECH_TIMEOUT)?;
                    if !should_continue {
                        break; // Session ended
                    }
                }
                Ok(None) => {
                    // No audio available, brief pause
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(e) => {
                    error!("❌ [{}] Audio read error in session {}: {}", self.client_id, session_id, e);
                    self.end_utterance_session(EosReason::Error)?;
                    break;
                }
            }
        }

        Ok(())
    }

    /// Process a single audio chunk during utterance capture
    fn process_utterance_chunk(&mut self, audio_chunk: &AudioChunk, session_start: Instant, pre_speech_timeout: Duration) -> Result<bool, OpenWakeWordError> {
        let session = match &mut self.current_session {
            Some(session) => session,
            None => return Ok(false), // No active session
        };

        // Convert audio chunk to f32 for VAD processing
        let f32_samples: Vec<f32> = audio_chunk.data.iter()
            .map(|&sample| sample as f32 / 32768.0)
            .collect();

        // Process in 512-sample chunks for VAD
        for chunk_512 in f32_samples.chunks(512) {
            if chunk_512.len() == 512 {
                let mut samples_array = [0.0f32; 512];
                samples_array.copy_from_slice(chunk_512);

                // Run VAD
                match self.vad_processor.process_chunk(&samples_array) {
                    Ok(AudioEvent::StartedAudio) => {
                        info!("🗣️ [{}] Speech started in session {}", self.client_id, session.session_id);
                        session.speech_detected = true;
                    }
                    Ok(AudioEvent::StoppedAudio) => {
                        if session.speech_detected {
                            info!("🔇 [{}] Speech ended in session {}", self.client_id, session.session_id);
                            self.end_utterance_session(EosReason::VadSilence)?;
                            return Ok(false); // End session
                        }
                    }
                    Ok(AudioEvent::Audio) => {
                        // Ongoing speech or silence - continue
                    }
                    Err(e) => {
                        warn!("⚠️ [{}] VAD error in session {}: {}", self.client_id, session.session_id, e);
                    }
                }
            }
        }

        // Stream this chunk to subscribers if we have speech or are in grace period
        if session.speech_detected || session_start.elapsed() < pre_speech_timeout {
            if let Some(ref server) = self.wakeword_server {
                let audio_message = wakeword_protocol::protocol::AudioChunk {
                    data: audio_chunk.data.iter().map(|&sample| sample as u8).collect(), // Convert i16 to u8 bytes
                    timestamp: SystemTime::now().duration_since(UNIX_EPOCH)
                        .unwrap_or_default().as_millis() as u64,
                    sequence_id: session.chunk_count,
                    session_id: session.session_id.clone(),
                };
                
                server.broadcast_message(crate::server::StreamingMessage::AudioChunk(audio_message));
                session.chunk_count += 1;
            }
        }

        // Timeout if no speech detected within grace period
        if !session.speech_detected && session_start.elapsed() > pre_speech_timeout {
            info!("⏰ [{}] No speech detected within timeout, ending session {}", self.client_id, session.session_id);
            self.end_utterance_session(EosReason::Timeout)?;
            return Ok(false);
        }

        Ok(true) // Continue session
    }

    /// End the current utterance capture session
    fn end_utterance_session(&mut self, reason: EosReason) -> Result<(), OpenWakeWordError> {
        let session = match self.current_session.take() {
            Some(session) => session,
            None => return Ok(()), // No active session
        };

        info!("🏁 [{}] Ending utterance session {} (reason: {:?}, {} chunks)", 
            self.client_id, session.session_id, reason, session.chunk_count);

        // Broadcast end of speech event
        if let Some(ref server) = self.wakeword_server {
            let eos_event = wakeword_protocol::protocol::EndOfSpeechEvent {
                session_id: session.session_id,
                timestamp: SystemTime::now().duration_since(UNIX_EPOCH)
                    .unwrap_or_default().as_millis() as u64,
                total_chunks: session.chunk_count,
                reason,
            };
            server.broadcast_message(crate::server::StreamingMessage::EndOfSpeech(eos_event));
        }

        Ok(())
    }
}


/// Convenience function to start wakeword detection with event server
pub fn start_wakeword_detection_with_server(
    audio_server_address: &str,
    wakeword_server_address: &str,
    model_names: Vec<String>,
    detection_threshold: f32,
) -> Result<(), OpenWakeWordError> {
    // Create and start the wakeword event server
    let wakeword_server = Arc::new(WakewordServer::new());
    wakeword_server.start(wakeword_server_address).map_err(|e| {
        OpenWakeWordError::IoError(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("Failed to start wakeword server: {}", e),
        ))
    })?;

    // Create the wakeword detection client
    let mut client = WakewordClient::new(audio_server_address, model_names, detection_threshold)?;
    
    // Enable event broadcasting
    client.set_wakeword_server(wakeword_server);

    // Start detection (blocking)
    client.start_detection()
}
