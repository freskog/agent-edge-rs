use agent_edge_rs::{
    audio_capture::AudioCaptureConfig,
    audio_sink::{CpalConfig, CpalSink},
    config::load_config,
    error::Result as EdgeResult,
    llm::integration::LLMIntegration,
    speech_producer::SpeechHub,
    stt::STTConfig,
    tts::{ElevenLabsTTS, TTSConfig},
    user_instruction::{Config as UserInstructionConfig, UserInstructionDetector},
};
use std::env;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> EdgeResult<()> {
    // Initialize logging
    env_logger::init();
    log::info!("🚀 Initializing agent-edge-rs");

    // Check for API keys
    if env::var("FIREWORKS_API_KEY").is_err() {
        eprintln!("❌ FIREWORKS_API_KEY environment variable not set");
        eprintln!("   Please set it with: export FIREWORKS_API_KEY=your_key_here");
        std::process::exit(1);
    }
    if env::var("GROQ_API_KEY").is_err() {
        eprintln!("❌ GROQ_API_KEY environment variable not set");
        eprintln!("   Please set it with: export GROQ_API_KEY=your_key_here");
        std::process::exit(1);
    }
    if env::var("ELEVENLABS_API_KEY").is_err() {
        eprintln!("❌ ELEVENLABS_API_KEY environment variable not set");
        eprintln!("   Please set it with: export ELEVENLABS_API_KEY=your_key_here");
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

    // Initialize LLM integration
    let api_config = load_config()?;
    let llm_integration = Arc::new(Mutex::new(LLMIntegration::new(&api_config).map_err(
        |e| agent_edge_rs::error::EdgeError::Unknown(format!("LLM integration failed: {}", e)),
    )?));
    log::info!("🤖 LLM integration initialized");

    // Initialize TTS
    let audio_sink = Arc::new(CpalSink::new(CpalConfig::default()).map_err(|e| {
        agent_edge_rs::error::EdgeError::Audio(format!("Audio sink failed: {}", e))
    })?);
    let tts = Arc::new(ElevenLabsTTS::new(
        api_config.elevenlabs_key().to_string(),
        TTSConfig::default(),
        audio_sink,
    ));
    log::info!("🔊 TTS initialized");

    println!("🎧 Listening for voice instructions...");
    println!("   Say the wakeword to start giving instructions");
    println!("   Press Ctrl+C to exit");

    // Track current processing for cancellation
    let mut current_processing: Option<(JoinHandle<()>, CancellationToken)> = None;

    // Main loop - parallel processing with cancellation
    loop {
        tokio::select! {
            instruction_result = detector.get_instruction() => {
                match instruction_result {
                    Ok(instruction) => {
                        // Cancel any current processing immediately
                        if let Some((handle, cancel_token)) = current_processing.take() {
                            log::info!("🛑 Cancelling current processing for new instruction");
                            cancel_token.cancel();
                            handle.abort(); // Don't wait for graceful shutdown
                        }

                        // Log the instruction
                        println!("✨ User instruction: \"{}\" (confidence: {:.3})",
                                instruction.text, instruction.confidence);
                        log::info!("Received instruction: {} (confidence: {:.3})",
                                  instruction.text, instruction.confidence);

                                                                                                // Start new processing (even empty transcript is processed)
                        let cancel_token = CancellationToken::new();
                        let llm_integration_clone = Arc::clone(&llm_integration);
                        let tts_clone = Arc::clone(&tts);
                        let transcript = instruction.text.clone();
                        let cancel_token_clone = cancel_token.clone();

                        let handle = tokio::spawn(async move {
                            let mut llm = llm_integration_clone.lock().await;
                            match llm.process_user_instruction(&transcript, cancel_token_clone.clone()).await {
                                Ok(Some(response)) => {
                                    println!("🗣️  Response: {}", response);
                                    log::info!("LLM response: {}", response);

                                    // Synthesize response with TTS
                                    log::info!("🔊 Starting TTS synthesis...");
                                    match tts_clone.synthesize(&response, cancel_token_clone).await {
                                        Ok(()) => {
                                            log::info!("✅ TTS synthesis completed successfully");
                                        }
                                        Err(e) => {
                                            if !e.to_string().contains("cancelled") {
                                                log::error!("TTS synthesis failed: {}", e);
                                                println!("❌ TTS Error: {}", e);
                                            }
                                        }
                                    }
                                }
                                Ok(None) => {
                                    // Silent execution (tool returned None)
                                    log::info!("Tool executed silently (no speech output)");
                                }
                                Err(e) => {
                                    // Only log non-cancellation errors
                                    if !e.to_string().contains("cancelled") {
                                        log::error!("Failed to process user instruction: {}", e);
                                        println!("❌ Error processing instruction: {}", e);
                                    }
                                }
                            }
                        });

                        current_processing = Some((handle, cancel_token));
                    }
                    Err(e) => {
                        log::error!("Failed to get user instruction: {}", e);
                        println!("❌ Error getting instruction: {}", e);

                        // Add a small delay before retrying to avoid tight error loops
                        tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
                    }
                }
            }

            // Handle completion of current processing
            Some(result) = async {
                if let Some((ref mut handle, _)) = current_processing {
                    Some(handle.await)
                } else {
                    None
                }
            } => {
                // Processing completed
                current_processing = None;
                if let Err(e) = result {
                    // Only log if it's not a cancellation
                    if !e.is_cancelled() {
                        log::error!("Processing task failed: {}", e);
                    }
                }
            }

            _ = tokio::signal::ctrl_c() => {
                log::info!("Received Ctrl+C, shutting down...");

                // Cancel any ongoing processing
                if let Some((handle, cancel_token)) = current_processing.take() {
                    cancel_token.cancel();
                    handle.abort();
                }

                println!("\n👋 Goodbye!");
                break;
            }
        }
    }

    Ok(())
}
