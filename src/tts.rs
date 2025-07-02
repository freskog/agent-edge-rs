use base64::{engine::general_purpose, Engine as _};
use futures_util::{SinkExt, StreamExt};
use once_cell::sync::OnceCell;
use serde_json::json;

use crate::audio_sink::{AudioError, AudioSink};
use std::sync::Arc;
use thiserror::Error;
use tokio::select;
use tokio::sync::{broadcast, mpsc};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tokio_util::sync::CancellationToken;

#[derive(Error, Debug, Clone)]
pub enum TTSError {
    #[error("API error: {status} - {message}")]
    ApiError { status: u16, message: String },

    #[error("WebSocket error: {0}")]
    WebSocket(String),

    #[error("Connection error: {0}")]
    Connection(String),

    #[error("Audio error: {0}")]
    Audio(#[from] AudioError),

    #[error("Session cancelled")]
    Cancelled,
}

impl From<reqwest::Error> for TTSError {
    fn from(err: reqwest::Error) -> Self {
        TTSError::Connection(err.to_string())
    }
}

impl From<tokio_tungstenite::tungstenite::Error> for TTSError {
    fn from(err: tokio_tungstenite::tungstenite::Error) -> Self {
        TTSError::WebSocket(err.to_string())
    }
}

#[derive(Debug, Clone)]
pub struct TTSConfig {
    pub voice_id: String,
    pub model: String,
    pub stability: f32,
    pub similarity_boost: f32,
    pub style: f32,
    pub use_speaker_boost: bool,
}

impl Default for TTSConfig {
    fn default() -> Self {
        Self {
            voice_id: "21m00Tcm4TlvDq8ikWAM".to_string(), // Rachel voice
            model: "eleven_multilingual_v2".to_string(),   // Better quality model
            stability: 0.75,                               // More stable voice
            similarity_boost: 0.85,                        // Better voice matching
            style: 0.35,                                   // Slight style boost for more natural speech
            use_speaker_boost: true,
        }
    }
}

#[derive(Debug)]
pub struct TTSResponse {
    pub audio_data: Vec<u8>,
    pub format: String,
}

pub struct ElevenLabsTTS {
    api_key: String,
    config: TTSConfig,
    sink: Arc<dyn AudioSink>,
}

// Global instance so other modules (e.g. LLM tools) can trigger speech without wiring TTS through every call.
static GLOBAL_TTS: OnceCell<Arc<ElevenLabsTTS>> = OnceCell::new();

impl ElevenLabsTTS {
    pub fn new(api_key: String, config: TTSConfig, sink: Arc<dyn AudioSink>) -> Self {
        Self {
            api_key,
            config,
            sink,
        }
    }

    /// Register this TTS engine as the global instance. Should be called once at start-up.
    pub fn set_global(instance: Arc<ElevenLabsTTS>) {
        // It is not an error if this is called twice; we only set on first call.
        let _ = GLOBAL_TTS.set(instance);
    }

    /// Get a reference to the global TTS instance if it has been registered.
    pub fn global() -> Option<&'static Arc<ElevenLabsTTS>> {
        GLOBAL_TTS.get()
    }

    /// Synthesize text to speech and play it through the configured audio sink
    pub async fn synthesize(&self, text: &str, cancel: CancellationToken) -> Result<(), TTSError> {
        log::debug!("TTS: Starting synthesis for text: {}", text);

        // Connect to WebSocket
        let ws_url = format!(
            "wss://api.elevenlabs.io/v1/text-to-speech/{}/stream-input?model_id={}&output_format=pcm_16000",
            self.config.voice_id, self.config.model
        );
        log::debug!("TTS: Connecting to WebSocket at {}", ws_url);

        let ws_stream = match connect_async(ws_url).await {
            Ok((ws_stream, _)) => {
                log::debug!("TTS: Successfully connected to WebSocket");
                ws_stream
            }
            Err(e) => {
                log::error!("TTS: Failed to connect to WebSocket: {}", e);
                return Err(TTSError::Connection(e.to_string()));
            }
        };

        let (mut write, mut read) = ws_stream.split();

        // Send initial configuration
        let bos_message = json!({
            "text": " ",  // Initial empty text
            "voice_settings": {
                "stability": self.config.stability,
                "similarity_boost": self.config.similarity_boost,
                "style": self.config.style,
                "use_speaker_boost": self.config.use_speaker_boost
            },
            "xi_api_key": self.api_key
        })
        .to_string();

        log::debug!("TTS: Sending initial configuration");
        // Send BOS message
        if let Err(e) = write.send(Message::Text(bos_message.into())).await {
            log::error!("TTS: Failed to send initial configuration: {}", e);
            return Err(TTSError::WebSocket(e.to_string()));
        }

        // Send text message
        let text_message = json!({
            "text": text,
            "try_trigger_generation": true
        })
        .to_string();

        log::debug!("TTS: Sending text for synthesis");
        if let Err(e) = write.send(Message::Text(text_message.into())).await {
            log::error!("TTS: Failed to send text message: {}", e);
            return Err(TTSError::WebSocket(e.to_string()));
        }

        // Send EOS message
        let eos_message = json!({
            "text": ""
        })
        .to_string();

        log::debug!("TTS: Sending end of stream message");
        if let Err(e) = write.send(Message::Text(eos_message.into())).await {
            log::error!("TTS: Failed to send EOS message: {}", e);
            return Err(TTSError::WebSocket(e.to_string()));
        }

        log::debug!("TTS: Starting audio stream processing");
        let mut total_audio_bytes = 0;
        let mut chunks_received = 0;

        // Process incoming audio data
        loop {
            select! {
                // Check for cancellation
                _ = cancel.cancelled() => {
                    log::debug!("TTS: Synthesis cancelled");
                    self.sink.stop().await?;
                    return Err(TTSError::Cancelled);
                }

                // Process WebSocket messages
                msg = read.next() => {
                    match msg {
                        Some(Ok(Message::Binary(audio_data))) => {
                            chunks_received += 1;
                            total_audio_bytes += audio_data.len();
                            log::debug!("TTS: Received chunk {} ({} bytes, total: {} bytes)", 
                                      chunks_received, audio_data.len(), total_audio_bytes);
                            
                            match self.sink.write(audio_data.as_slice()).await {
                                Ok(_) => log::debug!("TTS: Successfully wrote chunk {} to sink", chunks_received),
                                Err(e) => {
                                    log::error!("TTS: Failed to write audio chunk {} to sink: {}", chunks_received, e);
                                    return Err(e.into());
                                }
                            }
                        }
                        Some(Ok(Message::Text(text))) => {
                            log::debug!("TTS: Received text message: {}", text);
                            let text_str = text.to_string();
                            if text_str.contains("error") {
                                log::error!("TTS: Error in text message: {}", text_str);
                                return Err(TTSError::ApiError {
                                    status: 400,
                                    message: text_str,
                                });
                            } else if text_str.contains("\"audio\"") {
                                // Handle audio data in text message
                                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text_str) {
                                    if let Some(audio_b64) = json.get("audio").and_then(|a| a.as_str()) {
                                        chunks_received += 1;
                                        let audio_data = general_purpose::STANDARD.decode(audio_b64)
                                            .map_err(|e| TTSError::Audio(AudioError::Base64DecodeError(e.to_string())))?;
                                        total_audio_bytes += audio_data.len();
                                        log::debug!("TTS: Decoded base64 chunk {} ({} bytes, total: {} bytes)", 
                                                  chunks_received, audio_data.len(), total_audio_bytes);
                                        
                                        match self.sink.write(&audio_data).await {
                                            Ok(_) => log::debug!("TTS: Successfully wrote decoded chunk {} to sink", chunks_received),
                                            Err(e) => {
                                                log::error!("TTS: Failed to write decoded chunk {} to sink: {}", chunks_received, e);
                                                return Err(e.into());
                                            }
                                        }
                                    }
                                }
                            } else if text_str.contains("\"done\"") {
                                log::debug!("TTS: Synthesis complete - received {} chunks ({} bytes total)", 
                                         chunks_received, total_audio_bytes);
                                return Ok(());
                            }
                        }
                        Some(Ok(Message::Close(_))) => {
                            log::debug!("TTS: Received close frame, synthesis complete");
                            log::debug!("TTS: Final stats - {} chunks, {} bytes total", chunks_received, total_audio_bytes);
                            return Ok(());
                        }
                        Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) | Some(Ok(Message::Frame(_))) => {
                            // Ignore control frames
                            log::debug!("TTS: Received control frame (ping/pong/frame)");
                        }
                        Some(Err(e)) => {
                            log::error!("TTS: WebSocket error: {}", e);
                            return Err(TTSError::WebSocket(e.to_string()));
                        }
                        None => {
                            log::info!("TTS: WebSocket stream ended");
                            log::info!("TTS: Final stats - {} chunks, {} bytes total", chunks_received, total_audio_bytes);
                            return Ok(());
                        }
                    }
                }
            }
        }
    }

    /// Get available voices
    pub async fn get_voices(&self) -> Result<Vec<Voice>, TTSError> {
        let url = format!("https://api.elevenlabs.io/v1/voices");

        let response = reqwest::Client::new()
            .get(&url)
            .header("xi-api-key", &self.api_key)
            .send()
            .await?;

        let status = response.status();

        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(TTSError::ApiError {
                status: status.as_u16(),
                message: error_text,
            });
        }

        let response_text = response.text().await?;
        let json: serde_json::Value = serde_json::from_str(&response_text).map_err(|e| {
            TTSError::Audio(AudioError::InvalidJson(format!("Invalid JSON: {}", e)))
        })?;

        let voices_array = json["voices"]
            .as_array()
            .ok_or_else(|| TTSError::Audio(AudioError::MissingField("voices".to_string())))?;

        let mut voices = Vec::new();

        for voice_json in voices_array {
            if let Some(voice) = self.parse_voice(voice_json) {
                voices.push(voice);
            }
        }

        Ok(voices)
    }

    /// Parse a voice from JSON
    fn parse_voice(&self, voice_json: &serde_json::Value) -> Option<Voice> {
        Some(Voice {
            voice_id: voice_json["voice_id"].as_str()?.to_string(),
            name: voice_json["name"].as_str()?.to_string(),
            category: voice_json["category"].as_str().map(|s| s.to_string()),
            description: voice_json["description"].as_str().map(|s| s.to_string()),
            preview_url: voice_json["preview_url"].as_str().map(|s| s.to_string()),
        })
    }

    /// Set voice for future synthesis calls
    pub fn set_voice(&mut self, voice_id: String) {
        self.config.voice_id = voice_id;
    }

    /// Set voice settings
    pub fn set_voice_settings(&mut self, stability: f32, similarity_boost: f32, style: f32) {
        self.config.stability = stability.clamp(0.0, 1.0);
        self.config.similarity_boost = similarity_boost.clamp(0.0, 1.0);
        self.config.style = style.clamp(0.0, 1.0);
    }

    /// Convert MP3 audio to f32 samples for playback
    /// Note: This is a simplified implementation. For production use,
    /// consider using a proper audio decoding library like symphonia.
    pub fn mp3_to_samples(&self, _mp3_data: &[u8]) -> Result<(Vec<f32>, u32), TTSError> {
        // This is a placeholder implementation
        // In a real implementation, you would use an MP3 decoder
        Err(TTSError::Audio(AudioError::Mp3DecodingNotImplemented))
    }

    /// Save audio to file
    pub async fn save_audio(&self, audio_data: &[u8], filename: &str) -> Result<(), TTSError> {
        use tokio::fs;

        fs::write(filename, audio_data).await.map_err(|e| {
            TTSError::Audio(AudioError::FailedToSaveAudio(format!(
                "Failed to save audio: {}",
                e
            )))
        })?;

        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct Voice {
    pub voice_id: String,
    pub name: String,
    pub category: Option<String>,
    pub description: Option<String>,
    pub preview_url: Option<String>,
}

impl Voice {
    /// Popular voice presets
    pub fn rachel() -> String {
        "21m00Tcm4TlvDq8ikWAM".to_string()
    }

    pub fn drew() -> String {
        "29vD33N1CtxCmqQRPOHJ".to_string()
    }

    pub fn clyde() -> String {
        "2EiwWnXFnvU5JabPnv8n".to_string()
    }

    pub fn dave() -> String {
        "CYw3kZ02Hs0563khs1Fj".to_string()
    }

    pub fn fin() -> String {
        "D38z5RcWu1voky8WS1ja".to_string()
    }

    pub fn freya() -> String {
        "jsCqWAovK2LkecY7zXl4".to_string()
    }

    pub fn grace() -> String {
        "oWAxZDx7w5VEj9dCyTzz".to_string()
    }

    pub fn daniel() -> String {
        "onwK4e9ZLuTAKqWW03F9".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio_sink::{AudioSink, CpalConfig, CpalSink};
    use crate::config::ApiConfig;
    use std::sync::Arc;

    fn get_api_key_or_skip() -> String {
        match ApiConfig::load() {
            Ok(config) => config.elevenlabs_key().to_string(),
            Err(_) => {
                log::warn!("⚠️  Skipping ElevenLabs tests - API key not found");
                panic!("Test skipped - no API key");
            }
        }
    }

    #[tokio::test]
    async fn test_tts_synthesis() {
        let api_key = get_api_key_or_skip();
        let config = TTSConfig::default();
        let sink = match CpalSink::new(CpalConfig::default()) {
            Ok(sink) => Arc::new(sink) as Arc<dyn AudioSink>,
            Err(e) => {
                log::warn!(
                    "Audio device not available in test environment - this is expected: {}",
                    e
                );
                return;
            }
        };
        let tts = ElevenLabsTTS::new(api_key, config, Arc::clone(&sink));

        let cancel = CancellationToken::new();
        let result = tts.synthesize("Hello, this is a test.", cancel).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_tts_cancellation() {
        let api_key = get_api_key_or_skip();
        let config = TTSConfig::default();
        let sink = match CpalSink::new(CpalConfig::default()) {
            Ok(sink) => Arc::new(sink) as Arc<dyn AudioSink>,
            Err(e) => {
                log::warn!(
                    "Audio device not available in test environment - this is expected: {}",
                    e
                );
                return;
            }
        };
        let tts = ElevenLabsTTS::new(api_key, config, Arc::clone(&sink));

        let cancel = CancellationToken::new();
        let synthesis = tokio::spawn({
            let cancel = cancel.clone();
            async move {
                tts.synthesize(
                    "This is a longer text that should take some time to synthesize.",
                    cancel,
                )
                .await
            }
        });

        // Cancel after a short delay
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        cancel.cancel();

        let result = synthesis.await.unwrap();
        assert!(matches!(result, Err(TTSError::Cancelled)));

        // Verify sink is stopped by trying to write to it
        let write_result = sink.write(&[0, 0]).await;
        assert!(matches!(write_result, Err(AudioError::WriteError(_))));
    }
}

// ===== Streaming TTS Types =====

#[derive(Debug, Clone)]
pub enum AudioFormat {
    Pcm16000, // Best latency
    Pcm22050,
    Pcm24000,
    Pcm44100,
    Mp3_44100_128, // Fallback
}

impl AudioFormat {
    pub fn to_elevenlabs_format(&self) -> &'static str {
        match self {
            AudioFormat::Pcm16000 => "pcm_16000",
            AudioFormat::Pcm22050 => "pcm_22050",
            AudioFormat::Pcm24000 => "pcm_24000",
            AudioFormat::Pcm44100 => "pcm_44100",
            AudioFormat::Mp3_44100_128 => "mp3_44100_128",
        }
    }
}

#[derive(Debug, Clone)]
pub struct TTSStreamConfig {
    pub voice_id: String,
    pub model: String,
    pub output_format: AudioFormat,
    pub auto_mode: bool, // Reduces latency by disabling buffers
    pub stability: f32,
    pub similarity_boost: f32,
    pub style: f32,
    pub use_speaker_boost: bool,
}

impl Default for TTSStreamConfig {
    fn default() -> Self {
        Self {
            voice_id: "21m00Tcm4TlvDq8ikWAM".to_string(), // Rachel voice
            model: "eleven_flash_v2_5".to_string(),       // Fast model for streaming
            output_format: AudioFormat::Pcm16000,         // Best latency
            auto_mode: true,                              // Disable buffers for speed
            stability: 0.5,
            similarity_boost: 0.75,
            style: 0.0,
            use_speaker_boost: true,
        }
    }
}

#[derive(Debug, Clone)]
pub enum TTSEvent {
    StartedSpeaking,
    AudioChunk(Vec<u8>), // PCM samples ready for playback
    FinishedSpeaking,    // Signal for LED control
    Error(TTSError),
}

#[derive(Debug, Clone)]
pub enum TTSCommand {
    AddText(String),
    Finalize,
    Cancel,
}

pub struct TTSSession {
    pub events: broadcast::Receiver<TTSEvent>,
    pub control: TTSControl,
}

#[derive(Clone)]
pub struct TTSControl {
    command_sender: mpsc::Sender<TTSCommand>,
}

impl TTSControl {
    // Fire-and-forget mode: Queue text and continue
    pub async fn add_text(&self, text: &str) -> Result<(), TTSError> {
        self.command_sender
            .send(TTSCommand::AddText(text.to_string()))
            .await
            .map_err(|_| TTSError::Cancelled)?;
        Ok(())
    }

    // Blocking mode: Wait for TTS completion
    pub async fn speak_and_wait(&self, text: &str) -> Result<(), TTSError> {
        self.add_text(text).await?;
        self.finalize().await?;

        // For now, just wait a short time - we'll improve this later
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        Ok(())
    }

    pub async fn finalize(&self) -> Result<(), TTSError> {
        self.command_sender
            .send(TTSCommand::Finalize)
            .await
            .map_err(|_| TTSError::Cancelled)?;
        Ok(())
    }

    // Mid-sentence cancellation
    pub async fn cancel(&self) -> Result<(), TTSError> {
        self.command_sender
            .send(TTSCommand::Cancel)
            .await
            .map_err(|_| TTSError::Cancelled)?;
        Ok(())
    }
}

// ===== Streaming TTS Implementation =====

pub struct ElevenLabsStreamingTTS {
    api_key: String,
    base_url: String,
}

impl ElevenLabsStreamingTTS {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            base_url: "wss://api.elevenlabs.io/v1".to_string(),
        }
    }

    pub async fn start_session(&self, config: TTSStreamConfig) -> Result<TTSSession, TTSError> {
        let (event_tx, event_rx) = broadcast::channel(32);
        let (command_tx, command_rx) = mpsc::channel(16);

        // Start WebSocket connection and processing task
        let ws_url = format!(
            "{}/text-to-speech/{}/stream-input",
            self.base_url, config.voice_id
        );
        let api_key = self.api_key.clone();

        tokio::spawn(async move {
            if let Err(e) =
                Self::run_websocket_session(ws_url, api_key, config, event_tx.clone(), command_rx)
                    .await
            {
                let _ = event_tx.send(TTSEvent::Error(e));
            }
        });

        Ok(TTSSession {
            events: event_rx,
            control: TTSControl {
                command_sender: command_tx,
            },
        })
    }

    async fn run_websocket_session(
        ws_url: String,
        api_key: String,
        config: TTSStreamConfig,
        event_tx: broadcast::Sender<TTSEvent>,
        mut command_rx: mpsc::Receiver<TTSCommand>,
    ) -> Result<(), TTSError> {
        // Connect to WebSocket
        let (ws_stream, _) = connect_async(&ws_url).await?;
        let (mut ws_sink, mut ws_stream) = ws_stream.split();

        // Send initial handshake
        let init_message = json!({
            "text": " ",
            "voice_settings": {
                "stability": config.stability,
                "similarity_boost": config.similarity_boost,
                "style": config.style,
                "use_speaker_boost": config.use_speaker_boost
            },
            "xi_api_key": api_key,
            "model_id": config.model,
            "output_format": config.output_format.to_elevenlabs_format(),
            "auto_mode": config.auto_mode
        });

        ws_sink
            .send(Message::Text(init_message.to_string().into()))
            .await?;

        let mut finalized = false;

        loop {
            tokio::select! {
                // Handle commands from control interface
                command = command_rx.recv() => {
                    match command {
                        Some(TTSCommand::AddText(text)) => {
                            let message = json!({
                                "text": text,
                                "try_trigger_generation": true
                            });
                            ws_sink.send(Message::Text(message.to_string().into())).await?;
                        }
                        Some(TTSCommand::Finalize) => {
                            // Send empty text to finalize
                            let message = json!({
                                "text": ""
                            });
                            ws_sink.send(Message::Text(message.to_string().into())).await?;
                            finalized = true;
                        }
                        Some(TTSCommand::Cancel) => {
                            // Close WebSocket to cancel
                            ws_sink.close().await?;
                            break;
                        }
                        None => break, // Channel closed
                    }
                }

                // Handle WebSocket responses
                ws_message = ws_stream.next() => {
                    match ws_message {
                        Some(Ok(Message::Text(text))) => {
                            if let Ok(response) = serde_json::from_str::<serde_json::Value>(&text.to_string()) {
                                Self::handle_websocket_response(response, &event_tx)?;
                            }
                        }
                        Some(Ok(Message::Close(_))) => {
                            if finalized {
                                let _ = event_tx.send(TTSEvent::FinishedSpeaking);
                            }
                            break;
                        }
                        Some(Err(e)) => {
                            return Err(TTSError::WebSocket(e.to_string()));
                        }
                        None => break,
                        _ => {} // Ignore other message types
                    }
                }
            }
        }

        Ok(())
    }

    fn handle_websocket_response(
        response: serde_json::Value,
        event_tx: &broadcast::Sender<TTSEvent>,
    ) -> Result<(), TTSError> {
        if let Some(audio_b64) = response.get("audio").and_then(|a| a.as_str()) {
            // Decode base64 audio data
            let audio_data = general_purpose::STANDARD.decode(audio_b64).map_err(|e| {
                TTSError::Audio(AudioError::Base64DecodeError(format!(
                    "Base64 decode error: {}",
                    e
                )))
            })?;

            let _ = event_tx.send(TTSEvent::AudioChunk(audio_data));

            // Check if this is the final audio chunk
            if let Some(is_final) = response.get("isFinal").and_then(|f| f.as_bool()) {
                if is_final {
                    let _ = event_tx.send(TTSEvent::FinishedSpeaking);
                }
            }
        }

        Ok(())
    }

    pub async fn synthesize_streaming(
        &self,
        text: &str,
        config: TTSStreamConfig,
    ) -> Result<Vec<u8>, TTSError> {
        let session = self.start_session(config).await?;
        let mut audio_chunks = Vec::new();
        let mut events = session.events;

        // Send text and wait for completion
        session.control.speak_and_wait(text).await?;

        // Collect all audio chunks
        while let Ok(event) = events.recv().await {
            match event {
                TTSEvent::AudioChunk(chunk) => {
                    audio_chunks.extend(chunk);
                }
                TTSEvent::FinishedSpeaking => break,
                TTSEvent::Error(e) => return Err(e),
                _ => {}
            }
        }

        Ok(audio_chunks)
    }
}
