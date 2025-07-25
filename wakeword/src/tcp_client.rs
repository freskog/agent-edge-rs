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
        let model = Model::new(
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
                    if processing_time > Duration::from_millis(20) {
                        debug!(
                            "🐌 [{}] Slow processing: chunk {} took {:?}",
                            self.client_id, chunk_count, processing_time
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

                    // Check if it's a timeout
                    if e.to_string().contains("timed out") || e.to_string().contains("timeout") {
                        warn!("⏰ [{}] Audio chunk timeout detected", self.client_id);
                        if last_chunk_time.elapsed() > chunk_timeout {
                            error!(
                                "💀 [{}] No audio chunks received for {:?}, giving up",
                                self.client_id, chunk_timeout
                            );
                            break;
                        }
                    } else {
                        // For other errors, continue trying
                        warn!("🔄 [{}] Continuing after error...", self.client_id);
                        std::thread::sleep(Duration::from_millis(100));
                    }
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

        #[cfg(feature = "latency-diagnostics")]
        {
            // === NETWORK LATENCY DIAGNOSTICS ===
            // Track when chunks were captured vs received
            let network_latency = (SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64)
                .saturating_sub(chunk.timestamp_ms);

            // Track network jitter (variation in chunk arrival timing)
            self.chunk_receive_times
                .push_back((chunk.timestamp_ms, chunk_receive_time));
            if self.chunk_receive_times.len() > 10 {
                self.chunk_receive_times.pop_front();
            }

            // Calculate jitter from last few chunks
            if self.chunk_receive_times.len() >= 3 {
                let times: Vec<_> = self.chunk_receive_times.iter().collect();
                let mut intervals = Vec::new();
                for i in 1..times.len() {
                    let interval = times[i].1.duration_since(times[i - 1].1).as_millis() as i64;
                    intervals.push(interval);
                }

                if !intervals.is_empty() {
                    let mean_interval = intervals.iter().sum::<i64>() / intervals.len() as i64;
                    let jitter = intervals
                        .iter()
                        .map(|&i| (i - mean_interval).abs())
                        .max()
                        .unwrap_or(0);

                    debug!(
                        "[{}] Network: latency={}ms, jitter={}ms, chunk_interval={}ms",
                        self.client_id, network_latency, jitter, mean_interval
                    );
                }
            }

            debug!(
                "[{}] Processing audio chunk: {} bytes (timestamp: {}, network_latency: {}ms)",
                self.client_id,
                chunk.size_bytes(),
                chunk.timestamp_ms,
                network_latency
            );
        }

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

        #[cfg(feature = "latency-diagnostics")]
        {
            // Debug: Log sample statistics for comparison with test audio
            if !samples.is_empty() {
                let mean = samples.iter().map(|&x| x as f64).sum::<f64>() / samples.len() as f64;
                let min = samples.iter().min().unwrap_or(&0);
                let max = samples.iter().max().unwrap_or(&0);
                let rms = (samples.iter().map(|&x| (x as f64).powi(2)).sum::<f64>()
                    / samples.len() as f64)
                    .sqrt();

                debug!(
                    "[{}] Live audio stats: {} samples, mean={:.2}, min={}, max={}, RMS={:.2}",
                    self.client_id,
                    samples.len(),
                    mean,
                    min,
                    max,
                    rms
                );

                // Log first few samples for debugging
                debug!(
                    "[{}] First 8 samples: {:?}",
                    self.client_id,
                    &samples[..samples.len().min(8)]
                );
            }

            debug!(
                "[{}] Converted {} bytes to {} i16 samples in {:?}",
                self.client_id,
                chunk.data.len(),
                samples.len(),
                conversion_time
            );
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
            #[cfg(feature = "latency-diagnostics")]
            debug!("[{}] Buffer accumulation started", self.client_id);
        }

        const DETECTION_INTERVAL_MS: u64 = 160; // Run detection every 160ms for real-time processing

        let has_enough_samples = buffer_size_after >= DETECTION_WINDOW_SAMPLES;
        let enough_time_passed =
            last_detection_time.elapsed() >= Duration::from_millis(DETECTION_INTERVAL_MS);
        let should_detect = has_enough_samples && enough_time_passed;

        #[cfg(feature = "latency-diagnostics")]
        {
            // === DETECTION LATENCY DIAGNOSTICS ===
            if has_enough_samples && !enough_time_passed {
                let time_until_detection = Duration::from_millis(DETECTION_INTERVAL_MS)
                    .saturating_sub(last_detection_time.elapsed());
                debug!(
                    "[{}] Detection delayed by interval: {}ms remaining",
                    self.client_id,
                    time_until_detection.as_millis()
                );
            }
        }

        if should_detect {
            #[cfg(feature = "latency-diagnostics")]
            {
                // === AUDIO AGE ANALYSIS ===
                // Calculate how old the audio we're about to process is
                let buffer_fill_time = self
                    .buffer_fill_start
                    .map(|start| chunk_receive_time.duration_since(start))
                    .unwrap_or_default();

                // Estimate age of oldest sample in detection window
                let network_latency = (SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64)
                    .saturating_sub(chunk.timestamp_ms);
                let estimated_oldest_sample_age =
                    buffer_fill_time + Duration::from_millis(network_latency);

                info!(
                    "🔍 [{}] DETECTION START: buffer_time={}ms, est_audio_age={}ms, samples={}",
                    self.client_id,
                    buffer_fill_time.as_millis(),
                    estimated_oldest_sample_age.as_millis(),
                    buffer_size_after
                );
            }

            // Take the most recent window for detection
            let detection_samples = audio_buffer
                .iter()
                .skip(audio_buffer.len().saturating_sub(DETECTION_WINDOW_SAMPLES))
                .copied()
                .collect::<Vec<i16>>();

            #[cfg(feature = "latency-diagnostics")]
            {
                // Debug: Log detection window statistics
                let det_mean = detection_samples.iter().map(|&x| x as f64).sum::<f64>()
                    / detection_samples.len() as f64;
                let det_min = detection_samples.iter().min().unwrap_or(&0);
                let det_max = detection_samples.iter().max().unwrap_or(&0);
                let det_rms = (detection_samples
                    .iter()
                    .map(|&x| (x as f64).powi(2))
                    .sum::<f64>()
                    / detection_samples.len() as f64)
                    .sqrt();

                debug!(
                    "🔍 [{}] Detection window stats: {} samples, mean={:.2}, min={}, max={}, RMS={:.2}",
                    self.client_id,
                    detection_samples.len(),
                    det_mean,
                    det_min,
                    det_max,
                    det_rms
                );

                debug!(
                    "🔍 [{}] Running wake word detection (buffer: {} samples)",
                    self.client_id,
                    audio_buffer.len()
                );
            }

            // === MODEL INFERENCE TIMING ===
            let inference_start = Instant::now();
            match self.model.predict(&detection_samples, None, 1.0) {
                Ok(predictions) => {
                    let inference_time = inference_start.elapsed();
                    let total_detection_time = inference_start.elapsed();

                    #[cfg(feature = "latency-diagnostics")]
                    {
                        // === END-TO-END LATENCY CALCULATION ===
                        let buffer_fill_time = self
                            .buffer_fill_start
                            .map(|start| chunk_receive_time.duration_since(start))
                            .unwrap_or_default();
                        let network_latency = (SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64)
                            .saturating_sub(chunk.timestamp_ms);
                        let estimated_oldest_sample_age =
                            buffer_fill_time + Duration::from_millis(network_latency);
                        let end_to_end_latency = estimated_oldest_sample_age + total_detection_time;

                        info!(
                            "✅ [{}] DETECTION COMPLETE: inference={}ms, total={}ms, end_to_end={}ms",
                            self.client_id,
                            inference_time.as_millis(),
                            total_detection_time.as_millis(),
                            end_to_end_latency.as_millis()
                        );

                        // Warn about concerning latencies
                        if inference_time > Duration::from_millis(50) {
                            warn!(
                                "🐌 [{}] Slow inference: {}ms",
                                self.client_id,
                                inference_time.as_millis()
                            );
                        }
                        if end_to_end_latency > Duration::from_millis(500) {
                            warn!(
                                "🐌 [{}] High end-to-end latency: {}ms",
                                self.client_id,
                                end_to_end_latency.as_millis()
                            );
                        }
                        if network_latency > 100 {
                            warn!(
                                "🌐 [{}] High network latency: {}ms",
                                self.client_id, network_latency
                            );
                        }

                        debug!(
                            "[{}] Detection completed in {:?}: {:?}",
                            self.client_id, inference_time, predictions
                        );
                    }

                    #[cfg(not(feature = "latency-diagnostics"))]
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
                audio_buffer.drain(0..keep_from);
            }
        } else {
            #[cfg(feature = "latency-diagnostics")]
            {
                // === BUFFER STATUS LOGGING ===
                if buffer_size_after != self.last_buffer_size_logged
                    && buffer_size_after % 1000 == 0
                {
                    let progress = (buffer_size_after as f32 / DETECTION_WINDOW_SAMPLES as f32
                        * 100.0)
                        .min(100.0);
                    debug!(
                        "[{}] Buffer progress: {}/{} samples ({:.1}%)",
                        self.client_id, buffer_size_after, DETECTION_WINDOW_SAMPLES, progress
                    );
                    self.last_buffer_size_logged = buffer_size_after;
                }

                debug!(
                    "[{}] Skipping detection: buffer={}, since_last={:?}",
                    self.client_id,
                    audio_buffer.len(),
                    last_detection_time.elapsed()
                );
            }
        }

        #[cfg(feature = "latency-diagnostics")]
        {
            // === CONVERSION TIME WARNING ===
            if conversion_time > Duration::from_millis(5) {
                debug!(
                    "🐌 [{}] Slow conversion: {}ms",
                    self.client_id,
                    conversion_time.as_millis()
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
