use once_cell::sync::OnceCell;
use serde_json::json;
use std::sync::Arc;
use thiserror::Error;
use tungstenite;

#[derive(Error, Debug)]
pub enum TTSError {
    #[error("Connection error: {0}")]
    Connection(String),

    #[error("WebSocket error: {0}")]
    WebSocket(String),

    #[error("Audio error: {0}")]
    Audio(String),

    #[error("Session cancelled")]
    Cancelled,

    #[error("Synthesis error: {0}")]
    Synthesis(String),

    #[error("Playback error: {0}")]
    Playback(String),
}

impl From<tungstenite::Error> for TTSError {
    fn from(err: tungstenite::Error) -> Self {
        TTSError::WebSocket(err.to_string())
    }
}

pub struct ElevenLabsTTS {
    api_key: String,
    pub audio_address: String, // Add this field for blocking synthesis
}

// Global instance so other modules (e.g. LLM tools) can trigger speech without wiring TTS through every call.
static GLOBAL_TTS: OnceCell<Arc<ElevenLabsTTS>> = OnceCell::new();

impl ElevenLabsTTS {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            audio_address: String::new(), // Will be set later
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

    /// Blocking synthesis using tungstenite (not tokio-tungstenite)
    pub fn synthesize_blocking(&self, text: &str) -> Result<(), TTSError> {
        log::info!("🔊 Starting blocking TTS synthesis for: '{}'", text);

        if text.trim().is_empty() {
            log::info!("Empty text, skipping TTS");
            return Ok(());
        }

        // Connect to ElevenLabs WebSocket with 44.1kHz PCM format
        let voice_id = "21m00Tcm4TlvDq8ikWAM"; // Rachel voice
        let model = "eleven_flash_v2_5"; // Fast model for low latency
        let ws_url = format!(
            "wss://api.elevenlabs.io/v1/text-to-speech/{}/stream-input?model_id={}&output_format=pcm_44100",
            voice_id, model
        );

        log::debug!("🔗 Connecting to ElevenLabs WebSocket (blocking)");
        let (mut socket, _) = tungstenite::connect(ws_url)
            .map_err(|e| TTSError::Connection(format!("Failed to connect to ElevenLabs: {}", e)))?;

        // Send initial configuration
        let config_message = serde_json::json!({
            "text": " ", // Empty initial text
            "voice_settings": {
                "stability": 0.75,
                "similarity_boost": 0.85,
                "style": 0.35,
                "use_speaker_boost": true
            },
            "xi_api_key": self.api_key
        });

        log::debug!("📤 Sending TTS configuration");
        socket
            .send(tungstenite::Message::Text(config_message.to_string()))
            .map_err(|e| TTSError::Synthesis(format!("Failed to send config: {}", e)))?;

        // Send text for synthesis
        let text_message = serde_json::json!({
            "text": text,
            "try_trigger_generation": true
        });

        log::debug!("📤 Sending text for synthesis");
        socket
            .send(tungstenite::Message::Text(text_message.to_string()))
            .map_err(|e| TTSError::Synthesis(format!("Failed to send text: {}", e)))?;

        // Send end-of-stream marker
        let eos_message = serde_json::json!({
            "text": ""
        });

        socket
            .send(tungstenite::Message::Text(eos_message.to_string()))
            .map_err(|e| TTSError::Synthesis(format!("Failed to send EOS: {}", e)))?;

        // Connect to audio service for playback
        let stream_id = format!("tts_{}", uuid::Uuid::new_v4());
        let mut audio_client = audio_protocol::client::AudioClient::connect(&self.audio_address)
            .map_err(|e| {
                TTSError::Connection(format!("Failed to connect to audio service: {}", e))
            })?;

        log::debug!("🎵 Starting audio playback stream");
        let mut audio_chunks_sent = 0;

        // Process audio chunks from ElevenLabs and send to audio service
        loop {
            match socket.read() {
                Ok(tungstenite::Message::Binary(audio_data)) => {
                    audio_chunks_sent += 1;

                    log::debug!(
                        "📥 Received audio chunk {}: {} bytes",
                        audio_chunks_sent,
                        audio_data.len()
                    );

                    // Audio from ElevenLabs should already be PCM 44.1kHz mono s16le
                    match audio_client.play_audio_chunk(&stream_id, audio_data) {
                        Ok(result) => {
                            if !result.success {
                                log::warn!("Audio playback warning: {}", result.message);
                            }
                        }
                        Err(e) => {
                            log::error!("Failed to play audio chunk: {}", e);
                            return Err(TTSError::Playback(format!(
                                "Audio playback failed: {}",
                                e
                            )));
                        }
                    }
                }
                Ok(tungstenite::Message::Text(text_msg)) => {
                    log::debug!("📥 Received text message from ElevenLabs: {}", text_msg);
                    // Handle any status messages from ElevenLabs
                }
                Ok(tungstenite::Message::Close(_)) => {
                    log::debug!("🔚 ElevenLabs WebSocket closed");
                    break;
                }
                Err(e) => {
                    log::error!("❌ WebSocket error: {}", e);
                    return Err(TTSError::Connection(format!("WebSocket error: {}", e)));
                }
                _ => {
                    // Handle other message types if needed
                }
            }
        }

        // End the audio stream
        log::debug!("🔚 Ending audio stream, sent {} chunks", audio_chunks_sent);
        match audio_client.end_stream(&stream_id) {
            Ok(result) => {
                if result.success {
                    log::info!(
                        "✅ Audio stream completed: {} chunks played",
                        result.chunks_played
                    );
                } else {
                    log::warn!("Audio stream ended with warning: {}", result.message);
                }
            }
            Err(e) => {
                log::error!("Failed to end audio stream: {}", e);
                return Err(TTSError::Playback(format!("Failed to end stream: {}", e)));
            }
        }

        Ok(())
    }
}
