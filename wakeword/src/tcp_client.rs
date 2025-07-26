use crate::{error::OpenWakeWordError, Model};
use audio_protocol::{AudioChunk, AudioClient};
use log::{debug, error, info, warn};
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
        })
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
        let start_time = Instant::now();
        let mut last_chunk_time = Instant::now();
        let chunk_timeout = Duration::from_secs(5);
        let mut total_processing_time = Duration::new(0, 0);
        let mut detection_count = 0;

        // Detection frequency optimization - only run every N chunks
        let mut last_detection_time = Instant::now();

        // Simple synchronous loop - much cleaner than async streams!
        loop {
            match self.client.read_audio_chunk() {
                Ok(Some(chunk)) => {
                    chunk_count += 1;
                    last_chunk_time = Instant::now();

                    // Log every 10th chunk to reduce noise
                    if chunk_count % 10 == 0 {
                        let avg_processing_time = if detection_count > 0 {
                            total_processing_time.as_millis() / detection_count as u128
                        } else {
                            0
                        };

                        debug!(
                            "📥 [{}] Chunk #{}: {} bytes | {} detections | avg: {}ms/detection",
                            self.client_id,
                            chunk_count,
                            chunk.size_bytes(),
                            detection_count,
                            avg_processing_time
                        );
                        
                        // Buffer health metrics every 50 chunks (reduce log spam)
                        if chunk_count % 50 == 0 {
                            let buffer_utilization = (audio_buffer.len() as f32 / 16000.0 * 100.0).min(100.0);
                            debug!(
                                "📊 [{}] Buffer Health: {} samples ({:.1}% full, {:.1}s of audio)",
                                self.client_id, 
                                audio_buffer.len(), 
                                buffer_utilization,
                                audio_buffer.len() as f32 / 16000.0
                            );
                        }
                    }

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

                    // Check for slow processing
                    if processing_time > Duration::from_millis(80) {
                        debug!(
                            "🐌 [{}] Chunk processing too slow: {}ms (chunk=80ms) - falling behind real-time!",
                            self.client_id,
                            processing_time.as_millis()
                        );
                    }

                    // Detection count is now tracked inside process_audio_chunk when detection actually runs

                    // Log progress every 50 chunks with performance stats
                    if chunk_count % 50 == 0 {
                        let elapsed = start_time.elapsed();
                        let chunks_per_sec = chunk_count as f64 / elapsed.as_secs_f64();
                        let avg_processing =
                            total_processing_time.as_millis() / chunk_count as u128;

                        debug!("📊 [{}] Performance: {:.1} chunks/sec | avg: {}ms/chunk | {} detections", 
                              self.client_id, chunks_per_sec, avg_processing, detection_count);
                    }
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
        let conversion_time = conversion_start.elapsed();

        if samples.is_empty() {
            debug!("[{}] Received empty audio chunk, skipping", self.client_id);
            return Ok(());
        }

        // === DETAILED AUDIO DATA COMPARISON DEBUG ===
        if !samples.is_empty() {
            let mean = samples.iter().map(|&x| x as f64).sum::<f64>() / samples.len() as f64;
            let min = *samples.iter().min().unwrap_or(&0);
            let max = *samples.iter().max().unwrap_or(&0);
            let rms = (samples.iter().map(|&x| (x as f64).powi(2)).sum::<f64>() / samples.len() as f64).sqrt();
            debug!(
                "📊 RAW_AUDIO: len={}, mean={:.1}, min={}, max={}, rms={:.1}",
                samples.len(), mean, min, max, rms
            );
            
            // Log first 16 samples for comparison
            let first_samples: Vec<i16> = samples.iter().take(16).copied().collect();
            debug!("📊 RAW_SAMPLES: First 16 = {:?}", first_samples);
            
            // Check for audio corruption indicators
            if samples.iter().all(|&x| x == 0) {
                warn!("⚠️ AUDIO_WARNING: All samples are zero (silence)");
            }
            if samples.len() != chunk.data.len() / 2 {
                warn!("⚠️ AUDIO_WARNING: Sample count mismatch: {} vs {}", samples.len(), chunk.data.len() / 2);
            }
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
                    let total_detection_time = inference_start.elapsed();

                    debug!(
                        "[{}] Detection completed in {:?}ms",
                        self.client_id,
                        inference_time.as_millis()
                    );

                    self.handle_predictions(predictions);

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
    fn handle_predictions(&mut self, predictions: HashMap<String, f32>) {
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

                    // TODO: Add metrics, webhooks, or other actions here
                }
            } else {
                // Always log predictions to debug what's happening (increased precision)
                debug!(
                    "🔍 [{}] Detection: '{}' confidence {:.6} (threshold: {:.3})",
                    self.client_id, model_name, confidence, self.detection_threshold
                );
            }
        }
    }
}

/// Convenience function to create and start a wake word detection client
pub fn start_wakeword_detection(
    server_address: &str,
    model_names: Vec<String>,
    detection_threshold: f32,
) -> Result<(), OpenWakeWordError> {
    let mut client = WakewordClient::new(server_address, model_names, detection_threshold)?;
    client.start_detection()
}
