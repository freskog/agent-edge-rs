use agent_edge_rs::{
    detection::pipeline::{DetectionPipeline, OpenWakeWordConfig},
    error::Result,
    vad::VADConfig,
};
use log;

fn main() -> Result<()> {
    // Initialize logging - only show warnings and errors to keep stdout clean
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Warn)
        .format_timestamp_secs()
        .init();

    // Show startup banner
    println!("🎙️  WAKEWORD DETECTION SYSTEM STARTING...");
    println!("📊 Say: 'hey mycroft' to trigger detection");
    println!("🔧 Threshold: 0.5 confidence required");
    println!("⏹️  Press Ctrl+C to stop");
    println!("");

    println!("🔧 Initializing OpenWakeWord pipeline...");

    // Initialize the 3-stage OpenWakeWord pipeline
    let config = OpenWakeWordConfig::default();
    let mut pipeline = DetectionPipeline::new(
        "models/melspectrogram.tflite",
        "models/embedding_model.tflite",
        "models/hey_mycroft_v0.1.tflite",
        config,
    )?;

    println!("✅ OpenWakeWord pipeline ready (3-stage architecture)");

    // Enable WebRTC VAD for CPU optimization
    let vad_enabled = std::env::var("VAD_ENABLED").unwrap_or_else(|_| "true".to_string()) == "true";
    if vad_enabled {
        let mut vad_config = VADConfig::default();

        // Allow extra aggressive tuning via environment variables
        if let Ok(trigger_frames) = std::env::var("VAD_TRIGGER_FRAMES") {
            if let Ok(frames) = trigger_frames.parse::<usize>() {
                vad_config.speech_trigger_frames = frames;
                println!("🎤 VAD: Using custom trigger frames: {}", frames);
            }
        }

        if let Ok(silence_frames) = std::env::var("VAD_SILENCE_FRAMES") {
            if let Ok(frames) = silence_frames.parse::<usize>() {
                vad_config.silence_stop_frames = frames;
                println!("🎤 VAD: Using custom silence frames: {}", frames);
            }
        }

        pipeline.enable_vad(vad_config)?;
        println!("✅ WebRTC VAD enabled - will reduce CPU usage during silence");
    } else {
        println!("⚠️  WebRTC VAD disabled - processing all audio (original behavior)");
    }
    println!("");

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

                // Clean startup - no extra logging needed here

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

                                // Wakeword feedback already handled by println! above
                            } else if i % 50 == 0 {
                                // Every 4 seconds - more frequent feedback
                                let vad_info = if pipeline.is_vad_enabled() {
                                    let stats = pipeline.vad_stats();
                                    format!(" (VAD savings: {:.1}%)", stats.cpu_savings_percent)
                                } else {
                                    "".to_string()
                                };

                                println!(
                                    "🔄 Listening... (confidence: {:.4}){} - Try saying 'hey mycroft'",
                                    detection.confidence, vad_info
                                );
                                // Status feedback already handled by println! above
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

    println!("👋 Shutting down wakeword detection system");
    Ok(())
}

fn check_for_stop_input() -> bool {
    // Simplified for testing - always return false so it runs continuously
    false
}
