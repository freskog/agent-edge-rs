use crate::audio::AudioBuffer;
use crate::error::Result;
use crate::models::{WakewordConfig, WakewordDetection, WakewordDetector};
use std::time::Instant;

/// Configuration for the complete detection pipeline
#[derive(Debug, Clone)]
pub struct PipelineConfig {
    /// Wakeword detection configuration
    pub wakeword_config: WakewordConfig,
    /// Enable detailed logging for debugging
    pub debug_mode: bool,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
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
    /// Number of wakeword detections
    pub detections_count: u64,
    /// Total processing time for all chunks
    pub total_processing_time_ms: f64,
    /// Average processing time per chunk
    pub avg_processing_time_ms: f64,
    /// Last detection confidence
    pub last_confidence: f32,
}

/// Complete wakeword detection pipeline
///
/// This pipeline processes 80ms audio chunks (1280 samples at 16kHz) directly
/// through the integrated melspectrogram + wakeword detection models.
pub struct DetectionPipeline<'a> {
    wakeword_detector: WakewordDetector<'a>,
    config: PipelineConfig,
    stats: PipelineStats,
}

impl<'a> DetectionPipeline<'a> {
    /// Create a new detection pipeline
    pub fn new(config: PipelineConfig) -> Result<Self> {
        log::info!("Initializing wakeword detection pipeline");

        // Create the integrated wakeword detector
        let wakeword_detector = WakewordDetector::new(config.wakeword_config.clone())?;

        log::info!("Detection pipeline initialized successfully");
        log::info!(
            "Expected chunk size: {} samples",
            config.wakeword_config.chunk_size
        );

        Ok(Self {
            wakeword_detector,
            config,
            stats: PipelineStats {
                chunks_processed: 0,
                detections_count: 0,
                total_processing_time_ms: 0.0,
                avg_processing_time_ms: 0.0,
                last_confidence: 0.0,
            },
        })
    }

    /// Process an 80ms audio chunk through the complete pipeline
    ///
    /// # Arguments
    /// * `audio_samples` - Raw audio samples (must be exactly 1280 samples)
    ///
    /// # Returns
    /// * `Option<WakewordDetection>` - Detection result if processing succeeds
    pub fn process_chunk(&mut self, audio_samples: &[f32]) -> Result<WakewordDetection> {
        let start_time = Instant::now();

        // Process audio directly through the integrated detector
        let detection = self.wakeword_detector.process_audio(audio_samples)?;

        // Update statistics
        let processing_time = start_time.elapsed().as_secs_f64() * 1000.0;
        self.update_stats(&detection, processing_time);

        if self.config.debug_mode {
            log::debug!(
                "Processed chunk {}: {:.2}ms, confidence: {:.3}, detected: {}",
                self.stats.chunks_processed,
                processing_time,
                detection.confidence,
                detection.detected
            );
        }

        if detection.detected {
            log::info!("Wakeword detected! Confidence: {:.3}", detection.confidence);
        }

        Ok(detection)
    }

    /// Process audio from an AudioBuffer
    pub fn process_audio_buffer(
        &mut self,
        audio_buffer: &AudioBuffer,
    ) -> Result<WakewordDetection> {
        // AudioBuffer is Vec<f32>, so we can pass it directly
        self.process_chunk(audio_buffer)
    }

    /// Update internal statistics
    fn update_stats(&mut self, detection: &WakewordDetection, processing_time_ms: f64) {
        self.stats.chunks_processed += 1;
        self.stats.total_processing_time_ms += processing_time_ms;
        self.stats.avg_processing_time_ms =
            self.stats.total_processing_time_ms / self.stats.chunks_processed as f64;
        self.stats.last_confidence = detection.confidence;

        if detection.detected {
            self.stats.detections_count += 1;
        }
    }

    /// Process multiple audio chunks at once
    pub async fn process_chunks(
        &mut self,
        audio_chunks: Vec<&[f32]>,
    ) -> Result<Vec<WakewordDetection>> {
        let mut results = Vec::with_capacity(audio_chunks.len());

        for chunk in audio_chunks {
            let detection = self.process_chunk(chunk)?;
            if detection.detected {
                log::info!(
                    "Wakeword detected in batch processing! Confidence: {:.3}",
                    detection.confidence
                );
            }
            results.push(detection);
        }

        Ok(results)
    }

    /// Get current pipeline statistics
    pub fn stats(&self) -> &PipelineStats {
        &self.stats
    }

    /// Reset pipeline statistics
    pub fn reset_stats(&mut self) {
        self.stats = PipelineStats {
            chunks_processed: 0,
            detections_count: 0,
            total_processing_time_ms: 0.0,
            avg_processing_time_ms: 0.0,
            last_confidence: 0.0,
        };
        log::info!("Pipeline statistics reset");
    }

    /// Update wakeword detection threshold dynamically
    pub fn set_threshold(&mut self, _threshold: f32) {
        // Update the internal configuration
        // Note: The detector doesn't have a mutable set_threshold method,
        // so this would require recreating the detector or adding that method
        log::warn!("Dynamic threshold updates not yet implemented");
    }

    /// Get current detection threshold
    pub fn get_threshold(&self) -> f32 {
        self.wakeword_detector.config().confidence_threshold
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
        self.wakeword_detector.config().chunk_size
    }

    /// Get chunk duration in milliseconds
    pub fn chunk_duration_ms(&self) -> u32 {
        let samples = self.chunk_size_samples();
        let sample_rate = self.wakeword_detector.config().sample_rate;
        (samples as f64 / sample_rate as f64 * 1000.0) as u32
    }

    /// Get pipeline configuration
    pub fn config(&self) -> &PipelineConfig {
        &self.config
    }
}
