use agent_edge_rs::{
    audio_capture::AudioCaptureConfig,
    error::Result as EdgeResult,
    speech_producer::SpeechHub,
    stt::STTConfig,
    user_instruction::{Config as UserInstructionConfig, UserInstructionDetector},
};
use std::env;
use std::sync::Arc;

#[tokio::main]
async fn main() -> EdgeResult<()> {
    // Initialize logging
    env_logger::init();
    log::info!("🚀 Initializing agent-edge-rs");

    // Check for API key
    if env::var("FIREWORKS_API_KEY").is_err() {
        eprintln!("❌ FIREWORKS_API_KEY environment variable not set");
        eprintln!("   Please set it with: export FIREWORKS_API_KEY=your_key_here");
        std::process::exit(1);
    }

    // Initialize speech hub with dual-threshold approach for CPU efficiency
    // - 0.3 threshold for speech events (precise)
    // - 0.15 threshold for wakeword processing (lenient, ensures continuous audio)
    let speech_hub = Arc::new(SpeechHub::new(AudioCaptureConfig::default(), 0.3)?);
    log::info!("🎤 Speech hub initialized");

    // Create user instruction detector
    let instruction_config = UserInstructionConfig {
        stt_config: STTConfig::default(),
        wakeword_config: Default::default(),
    };

    let mut detector = UserInstructionDetector::new(instruction_config, speech_hub)?;
    log::info!("🔍 User instruction detector initialized");

    println!("🎧 Listening for voice instructions...");
    println!("   Say the wakeword to start giving instructions");
    println!("   Press Ctrl+C to exit");

    // Main loop - continuously get user instructions
    loop {
        tokio::select! {
            instruction_result = detector.get_instruction() => {
                match instruction_result {
                    Ok(instruction) => {
                        println!("✨ User instruction: \"{}\" (confidence: {:.3})",
                                instruction.text, instruction.confidence);
                        log::info!("Received instruction: {} (confidence: {:.3})",
                                  instruction.text, instruction.confidence);
                    }
                    Err(e) => {
                        log::error!("Failed to get user instruction: {}", e);
                        println!("❌ Error getting instruction: {}", e);

                        // Add a small delay before retrying to avoid tight error loops
                        tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
                    }
                }
            }
            _ = tokio::signal::ctrl_c() => {
                log::info!("Received Ctrl+C, shutting down...");
                println!("\n👋 Goodbye!");
                break;
            }
        }
    }

    Ok(())
}
