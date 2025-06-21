use crate::error::Result;
use crate::models::{
    embedding::EmbeddingModel, melspectrogram::MelspectrogramModel, wakeword::WakewordModel,
};
use crate::vad::{VADConfig, VADStats, WebRtcVAD};
use std::collections::VecDeque;

#[derive(Debug, Clone)]
pub struct DetectionResult {
    pub detected: bool,
    pub confidence: f32,
    pub timestamp: u128,
}

#[derive(Debug, Clone)]
pub struct OpenWakeWordConfig {
    pub chunk_size: usize,        // 1280 samples (80ms at 16kHz)
    pub detection_threshold: f32, // 0.5 default
}

impl Default for OpenWakeWordConfig {
    fn default() -> Self {
        Self {
            chunk_size: 1280, // 80ms at 16kHz
            detection_threshold: 0.5,
        }
    }
}

/// OpenWakeWord-style detection pipeline with CORRECTED approach:
/// - Generate embedding every 80ms chunk using sliding window of mel frames
/// - Accumulate embeddings in sliding window
/// - Make wakeword prediction every 80ms using sliding window of embeddings
/// - Optional WebRTC VAD for CPU optimization
pub struct DetectionPipeline<'a> {
    melspectrogram_model: MelspectrogramModel<'a>,
    embedding_model: EmbeddingModel<'a>,
    wakeword_model: WakewordModel<'a>,

    // Audio buffering for 80ms chunks
    audio_buffer: VecDeque<i16>,
    chunk_size: usize,

    // Mel frame buffering for embedding model (need ~76 frames for each embedding)
    mel_buffer: VecDeque<Vec<f32>>,

    // Embedding buffering for wakeword model (need 64 embeddings of 96 features each)
    embedding_buffer: VecDeque<Vec<f32>>,

    config: OpenWakeWordConfig,
    debug_mode: bool,

    // Debouncing to prevent repeated detections
    last_detection_time: Option<std::time::Instant>,
    debounce_duration: std::time::Duration,

    // WebRTC VAD for CPU optimization
    vad: Option<WebRtcVAD>,
    vad_stats: VADStats,
}

impl<'a> DetectionPipeline<'a> {
    pub fn new(
        melspectrogram_model_path: &str,
        embedding_model_path: &str,
        wakeword_model_path: &str,
        config: OpenWakeWordConfig,
    ) -> Result<Self> {
        log::info!("Initializing FINAL CORRECTED OpenWakeWord pipeline");

        let melspectrogram_model = MelspectrogramModel::new(melspectrogram_model_path)?;
        let embedding_model = EmbeddingModel::new(embedding_model_path)?;
        let wakeword_model = WakewordModel::new(wakeword_model_path)?;

        log::info!("🔍 CORRECTED OpenWakeWord Architecture (Matching Official):");
        log::info!("   ✅ Process each 80ms chunk → generate 1 embedding");
        log::info!("   ✅ Embedding uses sliding window of 76 mel frames");
        log::info!("   ✅ Accumulate embeddings in sliding window of 16");
        log::info!("   ✅ Wakeword prediction every 80ms using 16 embeddings");
        log::info!("   ✅ Model expects [1, 16, 96] = 1536 features (not 6144)");

        Ok(DetectionPipeline {
            melspectrogram_model,
            embedding_model,
            wakeword_model,
            audio_buffer: VecDeque::new(),
            chunk_size: config.chunk_size,
            mel_buffer: VecDeque::new(),
            embedding_buffer: VecDeque::new(),
            config,
            debug_mode: true,
            last_detection_time: None,
            debounce_duration: std::time::Duration::from_millis(1500), // 1.5 seconds debounce
            vad: None,                                                 // VAD is disabled by default
            vad_stats: VADStats::default(),
        })
    }

    pub fn process_audio_chunk(&mut self, audio_samples: &[i16]) -> Result<DetectionResult> {
        // Add samples to buffer
        self.audio_buffer.extend(audio_samples);

        let mut detection = DetectionResult {
            detected: false,
            confidence: 0.0,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis(),
        };

        // Process complete chunks - this triggers embedding generation and prediction every 80ms
        while self.audio_buffer.len() >= self.chunk_size {
            let chunk: Vec<i16> = self.audio_buffer.drain(..self.chunk_size).collect();

            // Check VAD if enabled - skip expensive processing if no voice activity
            let _should_process = if let Some(vad) = &mut self.vad {
                let start_time = std::time::Instant::now();
                // Use the new i16-based VAD interface to avoid double conversion
                let should_process = vad.should_process_audio_i16(&chunk)?;
                let processing_time = start_time.elapsed().as_millis() as u64;
                self.vad_stats.update(
                    should_process,
                    if should_process { processing_time } else { 0 },
                );

                if !should_process {
                    if self.debug_mode {
                        log::debug!(
                            "VAD: No voice activity detected - skipping expensive processing (CPU savings: {:.1}%)",
                            self.vad_stats.cpu_savings_percent
                        );
                    }
                    continue; // Skip expensive 3-stage processing
                }
                true
            } else {
                true // Always process if VAD is disabled
            };

            // Convert to f32 for model input (only after VAD check)
            let audio_chunk: Vec<f32> = chunk.iter().map(|&x| x as f32).collect();

            // Stage 1: Melspectrogram (80ms audio → mel features)
            let mel_features = self.melspectrogram_model.predict(&audio_chunk)?;

            // Each chunk gives us 5 time frames of 32 mel bins each (160 features total)
            if mel_features.len() != 160 {
                log::warn!("Expected 160 mel features, got {}", mel_features.len());
                continue;
            }

            // Convert to time frames: [160] → [5, 32]
            let mut time_frames = Vec::new();
            for frame_idx in 0..5 {
                let start = frame_idx * 32;
                let frame = mel_features[start..start + 32].to_vec();
                time_frames.push(frame);
            }

            // Add new frames to sliding window
            self.mel_buffer.extend(time_frames);

            // Maintain sliding window for embedding model
            while self.mel_buffer.len() > 80 {
                self.mel_buffer.pop_front();
            }

            if self.debug_mode {
                log::info!(
                    "Added 5 frames, buffer now has {} frames",
                    self.mel_buffer.len()
                );
            }

            // Generate embedding if we have enough frames
            if self.mel_buffer.len() >= 76 {
                // Use most recent 76 frames for embedding
                let start_idx = self.mel_buffer.len() - 76;
                let mut embedding_input = Vec::with_capacity(76 * 32);

                for i in start_idx..self.mel_buffer.len() {
                    embedding_input.extend(&self.mel_buffer[i]);
                }

                if embedding_input.len() != 2432 {
                    log::warn!(
                        "Embedding input size mismatch: got {}, expected 2432",
                        embedding_input.len()
                    );
                    continue;
                }

                // Stage 2: Embedding (76 mel frames → single embedding)
                let embeddings = self.embedding_model.predict(&embedding_input)?;

                if self.debug_mode {
                    log::info!(
                        "✅ Generated embedding {} using frames {}-{}, first 5 values: [{:.3}, {:.3}, {:.3}, {:.3}, {:.3}]",
                        embeddings.len(),
                        start_idx,
                        self.mel_buffer.len() - 1,
                        embeddings.get(0).unwrap_or(&0.0),
                        embeddings.get(1).unwrap_or(&0.0),
                        embeddings.get(2).unwrap_or(&0.0),
                        embeddings.get(3).unwrap_or(&0.0),
                        embeddings.get(4).unwrap_or(&0.0)
                    );
                }

                // Add embedding to sliding window
                self.embedding_buffer.push_back(embeddings);

                // Maintain sliding window of 16 embeddings (matching real OpenWakeWord)
                while self.embedding_buffer.len() > 16 {
                    self.embedding_buffer.pop_front();
                }

                // Stage 3: Wakeword prediction (every 80ms using sliding window of embeddings)
                if self.embedding_buffer.len() >= 1 {
                    let mut wakeword_input = Vec::with_capacity(1536);

                    if self.embedding_buffer.len() >= 16 {
                        // Use most recent 16 embeddings (not 64!)
                        let start_idx = self.embedding_buffer.len() - 16;
                        for i in start_idx..self.embedding_buffer.len() {
                            wakeword_input.extend(&self.embedding_buffer[i]);
                        }
                    } else {
                        // Zero-pad for early frames (like real OpenWakeWord)
                        // Zero-pad the missing slots FIRST
                        let missing_embeddings = 16 - self.embedding_buffer.len();
                        for _ in 0..missing_embeddings {
                            wakeword_input.extend(vec![0.0f32; 96]);
                        }

                        // Then add all available embeddings at the END
                        for embedding in &self.embedding_buffer {
                            wakeword_input.extend(embedding);
                        }
                    }

                    if wakeword_input.len() != 1536 {
                        log::warn!(
                            "Wakeword input size mismatch: got {}, expected 1536",
                            wakeword_input.len()
                        );
                        continue;
                    }

                    if self.debug_mode {
                        log::info!(
                            "Wakeword input prepared: {} embeddings, last 5 values: [{:.3}, {:.3}, {:.3}, {:.3}, {:.3}]",
                            self.embedding_buffer.len(),
                            wakeword_input.get(1531).unwrap_or(&0.0),
                            wakeword_input.get(1532).unwrap_or(&0.0),
                            wakeword_input.get(1533).unwrap_or(&0.0),
                            wakeword_input.get(1534).unwrap_or(&0.0),
                            wakeword_input.get(1535).unwrap_or(&0.0)
                        );
                    }

                    // This is the correct OpenWakeWord approach: prediction every 80ms!
                    detection.confidence = self.wakeword_model.predict(&wakeword_input)?;

                    if detection.confidence > self.config.detection_threshold {
                        // Check debouncing
                        let now = std::time::Instant::now();
                        let should_trigger = match self.last_detection_time {
                            Some(last_time) => {
                                now.duration_since(last_time) >= self.debounce_duration
                            }
                            None => true, // First detection
                        };

                        if should_trigger {
                            detection.detected = true;
                            self.last_detection_time = Some(now);
                            log::info!(
                                "🎉 WAKEWORD DETECTED! Confidence: {:.3} (using {} embeddings)",
                                detection.confidence,
                                self.embedding_buffer.len()
                            );
                        } else if self.debug_mode {
                            let remaining = self.debounce_duration.saturating_sub(
                                now.duration_since(self.last_detection_time.unwrap()),
                            );
                            log::info!(
                                "Debounced detection: {:.3} confidence ({}ms remaining)",
                                detection.confidence,
                                remaining.as_millis()
                            );
                        }
                    } else if self.debug_mode {
                        log::info!(
                            "Detection confidence: {:.4} (using {} embeddings)",
                            detection.confidence,
                            self.embedding_buffer.len()
                        );
                    }
                }
            } else if self.debug_mode {
                log::info!(
                    "📊 Collecting frames: {}/76 needed for embedding",
                    self.mel_buffer.len()
                );
            }
        }

        Ok(detection)
    }

    pub fn reset(&mut self) {
        self.audio_buffer.clear();
        self.mel_buffer.clear();
        self.embedding_buffer.clear();
        self.last_detection_time = None;
        if let Some(vad) = &mut self.vad {
            vad.reset();
        }
        self.vad_stats.reset();
        log::info!("Pipeline reset");
    }

    /// Enable WebRTC VAD with the given configuration
    pub fn enable_vad(&mut self, vad_config: VADConfig) -> Result<()> {
        self.vad = Some(WebRtcVAD::new(vad_config)?);
        self.vad_stats.reset();
        println!("🎤 WebRTC VAD enabled for CPU optimization");
        Ok(())
    }

    /// Disable WebRTC VAD
    pub fn disable_vad(&mut self) {
        self.vad = None;
        self.vad_stats.reset();
        println!("🎤 WebRTC VAD disabled");
    }

    /// Check if VAD is enabled
    pub fn is_vad_enabled(&self) -> bool {
        self.vad.is_some()
    }

    /// Get VAD statistics
    pub fn vad_stats(&self) -> &VADStats {
        &self.vad_stats
    }

    /// Get current VAD state (if enabled)
    pub fn is_speech_active(&self) -> bool {
        self.vad.as_ref().map_or(true, |vad| vad.is_speech_active())
    }
}
