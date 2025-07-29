use clap::Parser;
use std::thread;

use agent::blocking_stt::BlockingSTTService;
use agent::config::load_config;
use agent::services::llm::GroqLLMService;
use agent::services::stt::STTService;
use agent::services::tts::ElevenLabsTTSService;
use agent::services::{LLMService, STTService as STTServiceTrait, TTSService};
use anyhow::{Context, Result};
use audio_protocol::client::AudioClient;
use wakeword_protocol::client::WakewordClient;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// Address of the audio service
    #[arg(long, default_value = "127.0.0.1:8080")]
    audio_address: String,

    /// Address of the wakeword service  
    #[arg(long, default_value = "127.0.0.1:8081")]
    wakeword_address: String,
}

/// Main agent coordinator - now completely blocking
pub struct Agent {
    wakeword_client: WakewordClient,
    stt_service: STTService,
    llm_service: GroqLLMService,
    tts_service: ElevenLabsTTSService,
    audio_address: String, // Store for reconnection
}

impl Agent {
    pub fn new(args: Args) -> Result<Self, anyhow::Error> {
        // Load configuration
        let config = load_config().context("Failed to load configuration")?;

        // Initialize wakeword client
        let mut wakeword_client = WakewordClient::connect(&args.wakeword_address)
            .context("Failed to connect to wakeword service")?;
        log::info!("🎯 Wakeword client connected to {}", args.wakeword_address);

        // Subscribe to wakeword events
        wakeword_client
            .subscribe_wakeword()
            .context("Failed to subscribe to wakeword events")?;
        log::info!("👂 Subscribed to wakeword events");

        // Initialize blocking STT service with Fireworks API key
        let blocking_stt_service = BlockingSTTService::new(config.fireworks_key().to_string());
        let mut stt_service =
            STTService::new(blocking_stt_service).context("Failed to create STT service")?;
        log::info!("🎤 STT service initialized with blocking implementation");

        // Set up audio client for STT
        let audio_client = AudioClient::connect(&args.audio_address)
            .context("Failed to connect to audio service")?;
        stt_service.set_audio_client(audio_client);
        log::info!("🎧 Audio client connected for STT service");

        // Initialize LLM service
        let llm_service = GroqLLMService::new(&config).context("Failed to create LLM service")?;
        log::info!("🧠 LLM service initialized");

        // Initialize TTS service
        let tts_service = ElevenLabsTTSService::new(
            config.elevenlabs_key().to_string(),
            args.audio_address.clone(),
        )
        .context("Failed to create TTS service")?;
        log::info!("🔊 TTS service initialized");

        Ok(Self {
            wakeword_client,
            stt_service,
            llm_service,
            tts_service,
            audio_address: args.audio_address,
        })
    }

    /// Wait for a single wakeword detection (blocking)
    fn wait_for_wakeword(&mut self) -> Result<wakeword_protocol::WakewordEvent, anyhow::Error> {
        loop {
            match self.wakeword_client.read_wakeword_event() {
                Ok(Some(event)) => return Ok(event),
                Ok(None) => {
                    // No event, continue waiting
                    thread::sleep(std::time::Duration::from_millis(100));
                }
                Err(e) => {
                    return Err(anyhow::anyhow!("Wakeword client error: {}", e));
                }
            }
        }
    }

    /// Reconnect the audio client for STT service
    fn reconnect_audio_client(&mut self) -> Result<(), anyhow::Error> {
        log::info!("🔌 Reconnecting audio client to {}", self.audio_address);
        let audio_client = AudioClient::connect(&self.audio_address)
            .context("Failed to reconnect to audio service")?;
        self.stt_service.set_audio_client(audio_client);
        log::info!("🎧 Audio client reconnected for STT service");
        Ok(())
    }

    /// Run the agent (completely blocking)
    pub fn run(&mut self) -> Result<(), anyhow::Error> {
        log::info!("🤖 Starting agent in blocking mode");

        // Start audio buffering immediately
        self.stt_service
            .start_audio_buffering()
            .context("Failed to start audio buffering")?;

        log::info!("🎤 Audio buffering started");

        // Main event loop (blocking)
        loop {
            // Wait for wakeword (blocking)
            log::info!("👂 Waiting for wakeword...");

            match self.wait_for_wakeword() {
                Ok(wakeword_event) => {
                    log::info!(
                        "🎯 Wakeword detected: '{}' (confidence: {:.3})",
                        wakeword_event.model_name,
                        wakeword_event.confidence
                    );

                    // Process user instruction
                    if let Err(e) = self.process_instruction() {
                        log::error!("❌ Failed to process instruction: {}", e);
                    }
                }
                Err(e) => {
                    log::error!("❌ Wakeword detection failed: {}", e);
                    thread::sleep(std::time::Duration::from_secs(1));
                }
            }
        }
    }

    /// Process instruction (completely blocking)
    fn process_instruction(&mut self) -> Result<(), anyhow::Error> {
        // 1. STT (blocking) - this consumes the audio client
        let transcript = self
            .stt_service
            .transcribe_from_wakeword()
            .map_err(|e| anyhow::anyhow!("STT failed: {}", e))?;

        // 2. Reconnect audio client for next time
        self.reconnect_audio_client()
            .context("Failed to reconnect audio client")?;

        if transcript.trim().is_empty() {
            log::info!("📝 Empty transcript received, ending instruction");
            return Ok(());
        }

        log::info!("📝 Transcript: '{}'", transcript);

        // 3. LLM (blocking)
        let llm_response = self
            .llm_service
            .process(transcript)
            .map_err(|e| anyhow::anyhow!("LLM failed: {}", e))?;

        log::info!(
            "🧠 LLM returned {} tool calls",
            llm_response.tool_calls.len()
        );

        // 4. Execute tool calls (blocking)
        for tool_call in llm_response.tool_calls {
            if tool_call.name == "respond" {
                log::info!("💬 Executing respond tool: '{}'", tool_call.text);

                self.tts_service
                    .speak(tool_call.text)
                    .map_err(|e| anyhow::anyhow!("TTS failed: {}", e))?;
            } else {
                log::debug!("🔧 Ignoring unsupported tool call: {}", tool_call.name);
            }
        }

        log::info!("🔊 TTS completed");

        Ok(())
    }
}

fn main() -> Result<(), anyhow::Error> {
    env_logger::init();

    let args = Args::parse();
    log::info!("🚀 Starting agent with args: {:?}", args);

    let mut agent = Agent::new(args)?;
    agent.run()
}
