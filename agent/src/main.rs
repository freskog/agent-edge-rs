use clap::Parser;
use cpal::traits::{DeviceTrait, HostTrait};
use std::env;
use std::iter;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use agent::config;
use agent::llm::integration::LLMIntegration;
use agent::stt::STTConfig;
use agent::tts::{ElevenLabsTTS, TTSConfig};
use agent::types::StubAudioHub;
use agent::user_instruction::{Config as UserInstructionConfig, UserInstructionDetector};
use agent::{AudioSink, AudioSinkConfig, EdgeError, StubAudioSink};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Output audio device name (optional)
    #[arg(short, long)]
    output_device: Option<String>,

    /// List available audio devices and exit
    #[arg(short, long)]
    list_devices: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    env_logger::init();
    log::info!("🚀 Initializing agent-edge-rs");

    let args = Args::parse();

    // If --list-devices is specified, list devices and exit
    if args.list_devices {
        let host = cpal::default_host();
        println!("\nAvailable output devices:");
        match host.output_devices() {
            Ok(devices) => {
                for device in devices {
                    let name = device.name().unwrap_or_else(|e| format!("<error: {}>", e));
                    println!("  - {}", name);
                }
            }
            Err(e) => {
                println!("<error: {}>", e);
                return Ok(());
            }
        }
        println!("\nAvailable input devices:");
        match host.input_devices() {
            Ok(devices) => {
                for device in devices {
                    let name = device.name().unwrap_or_else(|e| format!("<error: {}>", e));
                    println!("  - {}", name);
                }
            }
            Err(e) => {
                println!("<error: {}>", e);
                return Ok(());
            }
        }
        return Ok(());
    }

    // Check for API keys
    if env::var("FIREWORKS_API_KEY").is_err() {
        log::error!("❌ FIREWORKS_API_KEY environment variable not set");
        log::error!("   Please set it with: export FIREWORKS_API_KEY=your_key_here");
        std::process::exit(1);
    }
    if env::var("GROQ_API_KEY").is_err() {
        log::error!("❌ GROQ_API_KEY environment variable not set");
        log::error!("   Please set it with: export GROQ_API_KEY=your_key_here");
        std::process::exit(1);
    }
    if env::var("ELEVENLABS_API_KEY").is_err() {
        log::error!("❌ ELEVENLABS_API_KEY environment variable not set");
        log::error!("   Please set it with: export ELEVENLABS_API_KEY=your_key_here");
        std::process::exit(1);
    }

    // Create speech hub for audio processing
    let speech_hub = Arc::new(
        StubAudioHub::new(agent::types::AudioCaptureConfig::default())
            .map_err(|e| anyhow::anyhow!(e))?,
    );
    log::info!("🎤 Speech hub initialized");
    log::debug!("🔧 Speech hub created successfully");

    // Create user instruction detector
    let instruction_config = UserInstructionConfig {
        stt_config: STTConfig::default(),
        wakeword_config: Default::default(),
    };

    log::debug!("🔧 Creating user instruction detector");
    let detector = Arc::new(Mutex::new(UserInstructionDetector::new(
        instruction_config,
        Arc::clone(&speech_hub),
    )?));
    log::info!("🔍 User instruction detector initialized");
    log::debug!("🔧 User instruction detector created successfully");

    // Initialize LLM integration
    log::debug!("🔧 Loading API config for LLM");
    let api_config = config::load_config().map_err(|e| EdgeError::Unknown(e.to_string()))?;
    log::debug!("🔧 Creating LLM integration");
    let llm_integration = Arc::new(Mutex::new(
        LLMIntegration::new(&api_config)
            .map_err(|e| EdgeError::Unknown(format!("LLM integration failed: {}", e)))?,
    ));
    log::info!("🤖 LLM integration initialized");
    log::debug!("🔧 LLM integration created successfully");

    // Initialize TTS
    log::debug!("🔧 Creating audio sink");
    let audio_sink = Arc::new(
        StubAudioSink::new(AudioSinkConfig {
            device_id: args.output_device,
            ..Default::default()
        })
        .map_err(|e| EdgeError::Audio(format!("Audio sink failed: {}", e)))?,
    );
    log::debug!("🔧 Creating TTS");
    let tts = Arc::new(ElevenLabsTTS::new(
        api_config.elevenlabs_key().to_string(),
        TTSConfig::default(),
        audio_sink.clone() as Arc<dyn AudioSink>,
    ));
    // Register as global so tools can access it directly
    ElevenLabsTTS::set_global(Arc::clone(&tts));
    log::info!("🔊 TTS initialized");
    log::debug!("🔧 TTS created successfully");

    log::info!("🎧 Listening for voice instructions...");
    log::info!("   Say the wakeword to start giving instructions");
    log::info!("   Press Ctrl+C to exit");
    log::debug!("🔧 About to start main instruction loop");

    // Track current processing for cancellation
    let mut current_processing: Option<(JoinHandle<()>, CancellationToken)> = None;

    log::debug!("🔄 Starting main instruction loop");

    // Main loop - parallel processing with cancellation
    loop {
        log::debug!("🔄 Main loop iteration - waiting for instruction");
        tokio::select! {
            instruction_result = async {
                log::debug!("🔄 Acquiring detector mutex for get_instruction");
                let mut detector = detector.lock().await;
                log::debug!("🔄 Calling get_instruction");
                detector.get_instruction().await
            } => {
                match instruction_result {
                    Ok(instruction) => {
                        // Cancel any current processing immediately
                        if let Some((handle, cancel_token)) = current_processing.take() {
                            log::info!("🛑 Cancelling current processing for new instruction");
                            cancel_token.cancel();
                            handle.abort(); // Don't wait for graceful shutdown
                        }

                        // Log the instruction
                        log::info!("✨ User instruction: \"{}\" (confidence: {:.3})",
                                instruction.text, instruction.confidence);

                        // Start new processing (even empty transcript is processed)
                        let cancel_token = CancellationToken::new();
                        let llm_integration_clone: Arc<Mutex<LLMIntegration>> = Arc::clone(&llm_integration);
                        let transcript = instruction.text.clone();
                        let cancel_token_clone = cancel_token.clone();

                        let handle = tokio::spawn(async move {
                            let mut llm = llm_integration_clone.lock().await;
                            match llm.process_user_instruction(&transcript, cancel_token_clone.clone()).await {
                                Ok(Some(response)) => {
                                    log::info!("🗣️  Response: {}", response);
                                }
                                Ok(None) => {
                                    log::info!("Tool executed silently (no speech output)");
                                }
                                Err(e) => {
                                    if !e.to_string().contains("cancelled") {
                                        log::error!("Failed to process user instruction: {}", e);
                                    }
                                }
                            }
                        });

                        current_processing = Some((handle, cancel_token));
                    }
                    Err(e) => {
                        log::error!("Failed to get user instruction: {}", e);
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
                    if !e.to_string().contains("cancelled") {
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
                break;
            }
        }
    }

    Ok(())
}
