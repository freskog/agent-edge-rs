use crate::audio::AudioBuffer;
use crate::error::Result;
use crate::models::{
    MelSpectrogramConfig, MelSpectrogramProcessor, WakewordConfig, WakewordDetection,
    WakewordDetector,
};
use std::time::Instant;

/// Configuration for the complete detection pipeline
#[derive(Debug, Clone)]
pub struct PipelineConfig {
    /// Melspectrogram processing configuration
    pub mel_config: MelSpectrogramConfig,
    /// Wakeword detection configuration
    pub wakeword_config: WakewordConfig,
    /// Enable detailed logging for debugging
    pub debug_mode: bool,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            mel_config: MelSpectrogramConfig::default(),
            wakeword_config: WakewordConfig::default(),
            debug_mode: false,
        }
    }
}

/// Statistics for monitoring pipeline performance
#[derive(Debug, Clone)]
pub struct PipelineStats {
    /// Total number of audio chunks processed
    pub chunks_processed: u64,
    /// Total number of mel frames generated
    pub frames_generated: u64,
    /// Number of frames currently buffered for wakeword detection
    pub frames_buffered: usize,
    /// Whether the wakeword detector has enough frames for detection
    pub is_ready_for_detection: bool,
    /// Total processing time for all chunks
    pub total_processing_time_ms: f64,
    /// Average processing time per chunk
    pub avg_processing_time_ms: f64,
}

/// Complete wakeword detection pipeline
pub struct DetectionPipeline {
    mel_processor: MelSpectrogramProcessor,
    wakeword_detector: WakewordDetector,
    config: PipelineConfig,
    stats: PipelineStats,
}

impl DetectionPipeline {
    /// Create a new detection pipeline
    pub fn new(config: PipelineConfig) -> Result<Self> {
        log::info!("Initializing wakeword detection pipeline");

        // Create melspectrogram processor
        let mel_processor = MelSpectrogramProcessor::new(config.mel_config.clone())?;

        // Get mel feature size from the processor
        let mel_feature_size = config.mel_config.n_mels;

        // Create wakeword detector with the mel feature size
        let wakeword_detector =
            WakewordDetector::new(config.wakeword_config.clone(), mel_feature_size)?;

        log::info!("Detection pipeline initialized successfully");

        Ok(Self {
            mel_processor,
            wakeword_detector,
            config,
            stats: PipelineStats {
                chunks_processed: 0,
                frames_generated: 0,
                frames_buffered: 0,
                is_ready_for_detection: false,
                total_processing_time_ms: 0.0,
                avg_processing_time_ms: 0.0,
            },
        })
    }

    /// Process an 80ms audio chunk through the complete pipeline
    pub fn process_chunk(
        &mut self,
        audio_chunk: &AudioBuffer,
    ) -> Result<Option<WakewordDetection>> {
        let start_time = Instant::now();

        // Step 1: Convert audio to mel spectrogram
        let mel_frame = self.mel_processor.process_chunk(audio_chunk)?;

        if self.config.debug_mode {
            log::debug!("Generated mel frame with {} features", mel_frame.len());
        }

        // Step 2: Process mel frame through wakeword detector
        let detection_result = self.wakeword_detector.process_frame(mel_frame)?;

        // Update statistics
        let processing_time = start_time.elapsed().as_secs_f64() * 1000.0;
        self.update_stats(processing_time);

        if self.config.debug_mode {
            let (buffered, total) = self.wakeword_detector.buffer_status();
            log::debug!(
                "Processed chunk {}: {}ms, buffer: {}/{}",
                self.stats.chunks_processed,
                processing_time,
                buffered,
                total
            );
        }

        Ok(detection_result)
    }

    /// Update internal statistics
    fn update_stats(&mut self, processing_time_ms: f64) {
        self.stats.chunks_processed += 1;
        self.stats.frames_generated += 1;
        self.stats.total_processing_time_ms += processing_time_ms;
        self.stats.avg_processing_time_ms =
            self.stats.total_processing_time_ms / self.stats.chunks_processed as f64;

        // Update buffer status
        let (buffered, total) = self.wakeword_detector.buffer_status();
        self.stats.frames_buffered = buffered;
        self.stats.is_ready_for_detection = buffered >= total;
    }

    /// Process multiple audio chunks at once
    pub async fn process_chunks(
        &mut self,
        audio_chunks: Vec<AudioBuffer>,
    ) -> Result<Vec<Option<WakewordDetection>>> {
        let mut results = Vec::with_capacity(audio_chunks.len());

        for chunk in &audio_chunks {
            if let Some(detection) = self.process_chunk(chunk)? {
                if detection.detected {
                    log::info!(
                        "Wakeword detected in batch processing! Confidence: {:.3}",
                        detection.confidence
                    );
                }
                results.push(Some(detection));
            } else {
                results.push(None);
            }
        }

        Ok(results)
    }

    /// Get current pipeline statistics
    pub fn stats(&self) -> &PipelineStats {
        &self.stats
    }

    /// Reset pipeline state (useful after detection or error)
    pub fn reset(&mut self) {
        self.wakeword_detector.reset_buffer();
        log::info!("Pipeline state reset");
    }

    /// Update wakeword detection threshold dynamically
    pub fn set_threshold(&mut self, threshold: f32) {
        self.wakeword_detector.set_threshold(threshold);
    }

    /// Get current detection threshold
    pub fn get_threshold(&self) -> f32 {
        self.wakeword_detector.threshold()
    }

    /// Enable or disable debug mode
    pub fn set_debug_mode(&mut self, enabled: bool) {
        self.config.debug_mode = enabled;
        log::info!(
            "Debug mode {}",
            if enabled { "enabled" } else { "disabled" }
        );
    }

    /// Get chunk size requirements for this pipeline
    pub fn chunk_size_samples(&self) -> usize {
        self.mel_processor.chunk_size_samples()
    }

    /// Get chunk duration in milliseconds
    pub fn chunk_duration_ms(&self) -> u32 {
        self.mel_processor.chunk_duration_ms()
    }
}
