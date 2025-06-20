use agent_edge_rs::{
    detection::pipeline::{DetectionPipeline, OpenWakeWordConfig},
    error::Result,
};
use log;

fn main() -> Result<()> {
    env_logger::init();

    // Show startup banner
    println!("🎙️  WAKEWORD DETECTION SYSTEM STARTING...");
    println!("📊 Say: 'hey mycroft' to trigger detection");
    println!("🔧 Threshold: 0.5 confidence required");
    println!("⏹️  Press Ctrl+C to stop");
    println!("");

    log::info!("Starting agent-edge-rs wakeword detection with proper OpenWakeWord architecture");

    // Initialize the 3-stage OpenWakeWord pipeline
    let config = OpenWakeWordConfig::default();
    let mut pipeline = DetectionPipeline::new(
        "models/melspectrogram.tflite",
        "models/embedding_model.tflite",
        "models/hey_mycroft_v0.1.tflite",
        config,
    )?;

    log::info!("✅ OpenWakeWord pipeline initialized with 3-stage architecture:");
    log::info!("   Stage 1: Melspectrogram (audio → mel features)");
    log::info!("   Stage 2: Embedding (mel features → speech embeddings)");
    log::info!("   Stage 3: Wakeword (embeddings → classification)");
    log::info!("");

    log::info!("🎯 Target: 'hey mycroft'");
    log::info!("📊 Threshold: 0.5");
    log::info!("⚡ Chunk size: 80ms (1280 samples)");
    log::info!("");

    // Try microphone capture first (if available)
    #[cfg(all(target_os = "linux", feature = "pulse"))]
    {
        use agent_edge_rs::audio::pulse_capture::{PulseAudioCapture, PulseAudioCaptureConfig};

        let config = PulseAudioCaptureConfig::default();
        match PulseAudioCapture::new(config) {
            Ok(mut audio_capture) => {
                // Start the capture
                if let Err(e) = audio_capture.start() {
                    println!("❌ MICROPHONE START FAILED!");
                    println!("   Error: {}", e);
                    println!("");
                    println!("🔧 TROUBLESHOOTING STEPS:");
                    println!(
                        "   1. Check if PulseAudio is running: systemctl --user status pulseaudio"
                    );
                    println!("   2. Start PulseAudio if needed: pulseaudio --start");
                    println!("   3. Check audio devices: pactl list sources short");
                    println!("   4. Add user to audio group: sudo usermod -a -G audio $USER");
                    println!("   5. Test basic recording: arecord -f cd -d 1 test.wav");
                    println!("");
                    println!("   Run ./debug-audio.sh for detailed diagnostics");
                    println!("");

                    return Err(e);
                }

                println!("🎤 MICROPHONE ACTIVE - Listening for 'hey mycroft'...");
                println!("   You should see activity indicators every few seconds below:");
                println!("");

                log::info!("🎤 Using real microphone input");
                log::info!("Press Ctrl+C to stop or say 'hey mycroft' to test detection!");
                log::info!("");

                // Main processing loop
                let mut i = 0;
                loop {
                    // Check for user input to stop
                    if check_for_stop_input() {
                        break;
                    }

                    // Process audio if available
                    match audio_capture.read_chunk() {
                        Ok(audio_chunk) => {
                            let detection = pipeline.process_audio_chunk(&audio_chunk)?;
                            i += 1;

                            if detection.detected {
                                println!("🚨🎉 WAKEWORD DETECTED! 🎉🚨");
                                println!("   Confidence: {:.3}", detection.confidence);
                                println!("   Say 'hey mycroft' again or press Ctrl+C to stop...");
                                println!("");

                                log::info!(
                                    "🎉 WAKEWORD DETECTED! Confidence: {:.3}",
                                    detection.confidence
                                );
                                log::info!("Say 'hey mycroft' again or press Ctrl+C to stop...");
                            } else if i % 50 == 0 {
                                // Every 4 seconds - more frequent feedback
                                println!(
                                    "🔄 Listening... (confidence: {:.4}) - Try saying 'hey mycroft'",
                                    detection.confidence
                                );
                                log::info!(
                                    "Listening... (confidence: {:.4}) - Say 'hey mycroft'!",
                                    detection.confidence
                                );
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
                println!("❌ MICROPHONE ACCESS FAILED!");
                println!("   Error: {}", e);
                println!("");
                println!("🔧 TROUBLESHOOTING STEPS:");
                println!(
                    "   1. Check if PulseAudio is running: systemctl --user status pulseaudio"
                );
                println!("   2. Start PulseAudio if needed: pulseaudio --start");
                println!("   3. Check audio devices: pactl list sources short");
                println!("   4. Add user to audio group: sudo usermod -a -G audio $USER");
                println!("   5. Test basic recording: arecord -f cd -d 1 test.wav");
                println!("");
                println!("   Run ./debug-audio.sh for detailed diagnostics");
                println!("");

                return Err(e);
            }
        }
    }

    #[cfg(not(all(target_os = "linux", feature = "pulse")))]
    {
        println!("❌ MICROPHONE SUPPORT NOT COMPILED");
        println!("   This build doesn't include PulseAudio support.");
        println!("   Please rebuild with: cargo build --release --features pulse");
        println!("");
        return Err(crate::error::EdgeError::Audio(
            "No microphone support compiled into this build".to_string(),
        )
        .into());
    }

    log::info!("Shutting down wakeword detection system");
    Ok(())
}

fn check_for_stop_input() -> bool {
    // Simplified for testing - always return false so it runs continuously
    false
}
