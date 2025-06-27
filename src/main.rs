use agent_edge_rs::{
    audio_capture::{AudioCapture, AudioCaptureConfig, PlatformAudioCapture},
    detection::pipeline::{DetectionPipeline, PipelineConfig},
    error::Result,
    vad::{VADConfig, WebRtcVAD},
};
use log;

fn main() -> Result<()> {
    // Initialize logging with environment variable support (newer env_logger API)
    env_logger::init();

    log::info!("Initializing agent-edge-rs");

    // Initialize the detection pipeline with static models
    let config = PipelineConfig::default();
    let mut pipeline = DetectionPipeline::new(config)?;

    // Initialize VAD for CPU efficiency
    let vad_config = VADConfig::default();
    let mut vad = WebRtcVAD::new(vad_config)?;
    log::info!("VAD initialized");

    let config = AudioCaptureConfig::default();
    match PlatformAudioCapture::new(config) {
        Ok(mut audio_capture) => {
            // Start the capture
            if let Err(e) = audio_capture.start() {
                log::error!("❌ MICROPHONE START FAILED! Error: {}", e);
                return Err(agent_edge_rs::error::EdgeError::Audio(e.to_string()));
            }

            log::info!("Microphone initialized");

            // Main processing loop
            log::info!("Starting main audio processing loop (80ms chunks, 1.5s debounce)");
            let mut chunk_count = 0;
            let mut last_log_time = std::time::Instant::now();
            let start_time = std::time::Instant::now();

            loop {
                // Process audio if available
                match audio_capture.read_chunk() {
                    Ok(audio_chunk) => {
                        chunk_count += 1;

                        // Log progress every 5 seconds
                        if last_log_time.elapsed() >= std::time::Duration::from_secs(5) {
                            log::info!(
                                "Audio processing: {} chunks received in last 5s",
                                chunk_count
                            );
                            chunk_count = 0;
                            last_log_time = std::time::Instant::now();
                        }

                        log::trace!("Read audio chunk: {} samples", audio_chunk.len());

                        // During startup (first 5 seconds), bypass VAD to build pipeline context faster
                        let should_process = if start_time.elapsed()
                            < std::time::Duration::from_secs(5)
                        {
                            log::debug!("Startup phase: bypassing VAD to build pipeline context");
                            true
                        } else {
                            vad.should_process_audio(&audio_chunk)?
                        };

                        if should_process {
                            if start_time.elapsed() >= std::time::Duration::from_secs(5) {
                                log::info!(
                                    "VAD detected speech, running wakeword detection (chunk #{})",
                                    chunk_count
                                );
                            }

                            // Convert i16 to f32 for the models only when needed
                            let audio_f32: Vec<f32> =
                                audio_chunk.iter().map(|&x| x as f32 / 32768.0).collect();

                            let detection = pipeline.process_audio_chunk(&audio_f32)?;

                            // Handle detection with callback
                            if detection.detected {
                                println!("🚨🎉 WAKEWORD DETECTED! 🎉🚨");
                                println!("   Confidence: {:.3}", detection.confidence);
                                println!("   Timestamp: {:?}", detection.timestamp);
                                println!("   Say 'hey mycroft' again or press Ctrl+C to stop...");
                                println!("");

                                // Reset pipeline state to prevent extra detections from same utterance
                                pipeline.reset();
                                log::info!("Pipeline state reset after detection");
                            }
                        } else {
                            log::trace!("VAD: No speech detected, skipping ML pipeline");
                        }

                        // Very brief sleep to prevent CPU spinning but maintain responsiveness
                        std::thread::sleep(std::time::Duration::from_millis(1));
                    }
                    Err(_) => {
                        // No audio available yet, sleep longer but not too long
                        log::trace!("No audio chunk available, waiting...");
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                }
            }
        }
        Err(e) => {
            log::error!("❌ MICROPHONE ACCESS FAILED! Error: {}", e);
            return Err(agent_edge_rs::error::EdgeError::Audio(e.to_string()));
        }
    }
}
