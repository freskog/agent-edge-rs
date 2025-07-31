use clap::Parser;
use std::collections::VecDeque;
use std::thread;

use agent::blocking_stt::BlockingSTTService;
use agent::config::load_config;
use agent::services::llm::GroqLLMService;
use agent::services::stt::STTService;
use agent::services::tts::ElevenLabsTTSService;
use agent::services::{LLMService, STTService as STTServiceTrait, TTSService};
use anyhow::{Context, Result};
use wakeword_protocol::client::{StreamingMessage, WakewordClient};
use wakeword_protocol::protocol::{AudioChunk, SubscriptionType};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// Address of the wakeword service  
    #[arg(long, default_value = "127.0.0.1:8081")]
    wakeword_address: String,
}

/// Conversation state for managing multi-turn interactions
#[derive(Debug, Clone, PartialEq)]
enum ConversationState {
    Idle,               // Waiting for wake word
    ProcessingWakeWord, // Got wake word, collecting utterance
    AwaitingFollowUp,   // In conversation, waiting for response without wake word
}

/// Main agent coordinator - now uses streaming wakeword protocol
pub struct Agent {
    wakeword_client: WakewordClient,
    stt_service: STTService,
    llm_service: GroqLLMService,
    tts_service: ElevenLabsTTSService,
    conversation_state: ConversationState,
    audio_buffer: VecDeque<AudioChunk>, // Buffer for collecting audio chunks
}

impl Agent {
    pub fn new(args: Args) -> Result<Self, anyhow::Error> {
        // Load configuration
        let config = load_config().context("Failed to load configuration")?;

        // Initialize wakeword client
        let mut wakeword_client = WakewordClient::connect(&args.wakeword_address)
            .context("Failed to connect to wakeword service")?;
        log::info!("🎯 Wakeword client connected to {}", args.wakeword_address);

        // Subscribe to utterance streaming (wake word + audio)
        wakeword_client
            .subscribe_utterance(SubscriptionType::WakewordPlusUtterance)
            .context("Failed to subscribe to utterance streaming")?;
        log::info!("👂 Subscribed to utterance streaming");

        // Initialize blocking STT service with Fireworks API key
        let blocking_stt_service = BlockingSTTService::new(config.fireworks_key().to_string());
        let stt_service =
            STTService::new(blocking_stt_service).context("Failed to create STT service")?;
        log::info!("🎤 STT service initialized with blocking implementation");

        // The STT service no longer needs an audio client - it will receive audio chunks from wakeword streaming

        // Initialize LLM service
        let llm_service = GroqLLMService::new(&config).context("Failed to create LLM service")?;
        log::info!("🧠 LLM service initialized");

        // Initialize TTS service - it will need its own audio connection for playback
        let tts_service = ElevenLabsTTSService::new(
            config.elevenlabs_key().to_string(),
            "127.0.0.1:8080".to_string(), // Default audio service address for TTS playback
        )
        .context("Failed to create TTS service")?;
        log::info!("🔊 TTS service initialized");

        Ok(Self {
            wakeword_client,
            stt_service,
            llm_service,
            tts_service,
            conversation_state: ConversationState::Idle,
            audio_buffer: VecDeque::new(),
        })
    }

    /// Wait for streaming messages and handle them based on conversation state
    fn wait_for_streaming_message(&mut self) -> Result<StreamingMessage, anyhow::Error> {
        loop {
            match self.wakeword_client.read_streaming_message() {
                Ok(Some(message)) => return Ok(message),
                Ok(None) => {
                    // No message, continue waiting
                    thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(e) => {
                    return Err(anyhow::anyhow!("Wakeword client error: {}", e));
                }
            }
        }
    }

    /// Run the agent (completely blocking)
    pub fn run(&mut self) -> Result<(), anyhow::Error> {
        log::info!("🤖 Starting agent with streaming wakeword protocol");

        // No need to start separate audio buffering - we get audio through wakeword streaming

        // Main event loop (blocking)
        loop {
            match self.conversation_state {
                ConversationState::Idle => {
                    // Wait for wake word + utterance
                    log::info!("👂 Waiting for wakeword and utterance...");
                    self.handle_wakeword_utterance_stream()?;
                }
                ConversationState::AwaitingFollowUp => {
                    // Subscribe to utterance-only mode for follow-up questions
                    log::info!("🔄 Switching to utterance-only mode for follow-up");
                    self.wakeword_client
                        .unsubscribe_utterance() // First unsubscribe from current
                        .context("Failed to unsubscribe from current subscription")?;

                    self.wakeword_client
                        .subscribe_utterance(SubscriptionType::UtteranceOnly)
                        .context("Failed to subscribe to utterance-only mode")?;

                    self.handle_utterance_only_stream()?;
                }
                ConversationState::ProcessingWakeWord => {
                    // This state should not be reached in the main loop
                    log::warn!("⚠️ Unexpected ProcessingWakeWord state in main loop");
                    self.conversation_state = ConversationState::Idle;
                }
            }
        }
    }

    /// Handle wake word + utterance streaming
    fn handle_wakeword_utterance_stream(&mut self) -> Result<(), anyhow::Error> {
        self.audio_buffer.clear();
        let mut got_wakeword = false;
        let mut session_id: Option<String> = None;

        loop {
            let message = self.wait_for_streaming_message()?;

            match message {
                StreamingMessage::WakewordEvent(event) => {
                    log::info!(
                        "🎯 Wakeword detected: '{}' (confidence: {:.3})",
                        event.model_name,
                        event.confidence
                    );
                    got_wakeword = true;
                    self.conversation_state = ConversationState::ProcessingWakeWord;
                }
                StreamingMessage::UtteranceSessionStarted(session) => {
                    log::info!("🎤 Utterance session started: {}", session.session_id);
                    session_id = Some(session.session_id);
                }
                StreamingMessage::AudioChunk(chunk) => {
                    if got_wakeword {
                        log::debug!(
                            "🎵 Received audio chunk {} for session {}",
                            chunk.sequence_id,
                            chunk.session_id
                        );
                        self.audio_buffer.push_back(chunk);
                    }
                }
                StreamingMessage::EndOfSpeech(eos_event) => {
                    log::info!(
                        "🏁 End of speech for session {} (reason: {:?})",
                        eos_event.session_id,
                        eos_event.reason
                    );

                    if got_wakeword && !self.audio_buffer.is_empty() {
                        // Process the complete utterance
                        self.process_utterance_chunks()?;
                        return Ok(()); // Return to main loop
                    } else {
                        log::warn!("⚠️ End of speech without wake word or audio chunks");
                        self.conversation_state = ConversationState::Idle;
                        return Ok(());
                    }
                }
                StreamingMessage::Error(error) => {
                    log::error!("❌ Streaming error: {}", error);
                    self.conversation_state = ConversationState::Idle;
                    return Err(anyhow::anyhow!("Streaming error: {}", error));
                }
            }
        }
    }

    /// Handle utterance-only streaming (for follow-up questions)
    fn handle_utterance_only_stream(&mut self) -> Result<(), anyhow::Error> {
        self.audio_buffer.clear();
        let mut session_id: Option<String> = None;

        // Set a timeout for follow-up responses
        let timeout = std::time::Duration::from_secs(30);
        let start_time = std::time::Instant::now();

        loop {
            if start_time.elapsed() > timeout {
                log::info!("⏰ Follow-up timeout reached, returning to idle mode");
                self.conversation_state = ConversationState::Idle;
                // Switch back to wake word + utterance mode
                self.wakeword_client.unsubscribe_utterance()?;
                self.wakeword_client
                    .subscribe_utterance(SubscriptionType::WakewordPlusUtterance)?;
                return Ok(());
            }

            let message = self.wait_for_streaming_message()?;

            match message {
                StreamingMessage::UtteranceSessionStarted(session) => {
                    log::info!(
                        "🎤 Follow-up utterance session started: {}",
                        session.session_id
                    );
                    session_id = Some(session.session_id);
                }
                StreamingMessage::AudioChunk(chunk) => {
                    log::debug!(
                        "🎵 Received follow-up audio chunk {} for session {}",
                        chunk.sequence_id,
                        chunk.session_id
                    );
                    self.audio_buffer.push_back(chunk);
                }
                StreamingMessage::EndOfSpeech(eos_event) => {
                    log::info!(
                        "🏁 End of follow-up speech for session {} (reason: {:?})",
                        eos_event.session_id,
                        eos_event.reason
                    );

                    if !self.audio_buffer.is_empty() {
                        // Process the follow-up utterance
                        self.process_utterance_chunks()?;
                        return Ok(()); // Return to main loop
                    } else {
                        log::warn!("⚠️ End of speech without audio chunks");
                        self.conversation_state = ConversationState::Idle;
                        return Ok(());
                    }
                }
                StreamingMessage::WakewordEvent(_) => {
                    // Shouldn't receive wake word events in utterance-only mode
                    log::warn!("⚠️ Unexpected wake word event in utterance-only mode");
                }
                StreamingMessage::Error(error) => {
                    log::error!("❌ Follow-up streaming error: {}", error);
                    self.conversation_state = ConversationState::Idle;
                    return Err(anyhow::anyhow!("Follow-up streaming error: {}", error));
                }
            }
        }
    }

    /// Process collected audio chunks through STT and continue with LLM/TTS
    fn process_utterance_chunks(&mut self) -> Result<(), anyhow::Error> {
        log::info!(
            "🎤 Processing {} audio chunks for STT",
            self.audio_buffer.len()
        );

        // Convert audio chunks to the format expected by STT service
        let audio_chunks: Vec<_> = self.audio_buffer.drain(..).collect();

        // Process through STT
        let transcript = self
            .stt_service
            .transcribe_from_chunks(audio_chunks)
            .map_err(|e| anyhow::anyhow!("STT failed: {}", e))?;

        if transcript.trim().is_empty() {
            log::info!("📝 Empty transcript received, ending instruction");
            self.conversation_state = ConversationState::Idle;
            return Ok(());
        }

        log::info!("📝 Transcript: '{}'", transcript);

        // Process through LLM
        let llm_response = self
            .llm_service
            .process(transcript)
            .map_err(|e| anyhow::anyhow!("LLM failed: {}", e))?;

        log::info!(
            "🧠 LLM returned {} tool calls",
            llm_response.tool_calls.len()
        );

        // Execute tool calls (TTS)
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

        // Set state for potential follow-up questions
        self.conversation_state = ConversationState::AwaitingFollowUp;

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
