use agent_edge_rs::{
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

    use agent_edge_rs::audio::pulse_capture::{PulseAudioCapture, PulseAudioCaptureConfig};

    let config = PulseAudioCaptureConfig::default();
    match PulseAudioCapture::new(config) {
        Ok(mut audio_capture) => {
            // Start the capture
            if let Err(e) = audio_capture.start() {
                log::error!("❌ MICROPHONE START FAILED! Error: {}", e);
                return Err(e);
            }

            log::info!("Microphone initialized");

            // Main processing loop
            loop {
                // Process audio if available
                match audio_capture.read_chunk() {
                    Ok(audio_chunk) => {
                        // First run VAD on the raw i16 audio
                        let should_process = vad.should_process_audio(&audio_chunk)?;

                        if should_process {
                            log::debug!("VAD detected speech, running wakeword detection");

                            // Convert i16 to f32 for the models only when needed
                            let audio_f32: Vec<f32> =
                                audio_chunk.iter().map(|&x| x as f32 / 32768.0).collect();

                            let detection = pipeline.process_audio_chunk(&audio_f32)?;

                            // Handle detection with callback
                            if detection.detected {
                                println!("🚨🎉 WAKEWORD DETECTED! 🎉🚨");
                                println!("   Confidence: {:.3}", detection.confidence);
                                println!("   Say 'hey mycroft' again or press Ctrl+C to stop...");
                                println!("");
                            }
                        } else {
                            log::trace!("VAD: No speech detected, skipping ML pipeline");
                        }
                    }
                    Err(_) => {
                        // No audio available yet, continue
                    }
                }

                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        Err(e) => {
            log::error!("❌ MICROPHONE ACCESS FAILED! Error: {}", e);
            return Err(e);
        }
    }
}
