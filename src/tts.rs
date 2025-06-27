use reqwest::Client;
use serde_json::json;
use std::time::Duration;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum TTSError {
    #[error("HTTP request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("API error: {status} - {message}")]
    ApiError { status: u16, message: String },
    #[error("Audio processing error: {0}")]
    AudioProcessing(String),
    #[error("Configuration error: {0}")]
    Config(String),
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
            model: "eleven_multilingual_v2".to_string(),
            stability: 0.5,
            similarity_boost: 0.75,
            style: 0.0,
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
    client: Client,
    api_key: String,
    base_url: String,
    config: TTSConfig,
}

impl ElevenLabsTTS {
    pub fn new(api_key: String) -> Self {
        Self::with_config(api_key, TTSConfig::default())
    }

    pub fn with_config(api_key: String, config: TTSConfig) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            client,
            api_key,
            base_url: "https://api.elevenlabs.io/v1".to_string(),
            config,
        }
    }

    /// Generate speech from text
    pub async fn synthesize(&self, text: &str) -> Result<TTSResponse, TTSError> {
        self.synthesize_with_voice(text, &self.config.voice_id)
            .await
    }

    /// Generate speech with specific voice ID
    pub async fn synthesize_with_voice(
        &self,
        text: &str,
        voice_id: &str,
    ) -> Result<TTSResponse, TTSError> {
        let url = format!("{}/text-to-speech/{}", self.base_url, voice_id);

        let payload = json!({
            "text": text,
            "model_id": self.config.model,
            "voice_settings": {
                "stability": self.config.stability,
                "similarity_boost": self.config.similarity_boost,
                "style": self.config.style,
                "use_speaker_boost": self.config.use_speaker_boost
            }
        });

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .header("Accept", "audio/mpeg")
            .json(&payload)
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

        let audio_data = response.bytes().await?.to_vec();

        Ok(TTSResponse {
            audio_data,
            format: "mp3".to_string(),
        })
    }

    /// Get available voices
    pub async fn get_voices(&self) -> Result<Vec<Voice>, TTSError> {
        let url = format!("{}/voices", self.base_url);

        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
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
        let json: serde_json::Value = serde_json::from_str(&response_text)
            .map_err(|e| TTSError::AudioProcessing(format!("Invalid JSON: {}", e)))?;

        let voices_array = json["voices"]
            .as_array()
            .ok_or_else(|| TTSError::AudioProcessing("Missing 'voices' field".to_string()))?;

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
        Err(TTSError::AudioProcessing(
            "MP3 decoding not implemented. Use an audio library like symphonia.".to_string(),
        ))
    }

    /// Save audio to file
    pub async fn save_audio(&self, audio_data: &[u8], filename: &str) -> Result<(), TTSError> {
        use tokio::fs;

        fs::write(filename, audio_data)
            .await
            .map_err(|e| TTSError::AudioProcessing(format!("Failed to save audio: {}", e)))?;

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

    #[test]
    fn test_config_defaults() {
        let config = TTSConfig::default();
        assert_eq!(config.voice_id, "21m00Tcm4TlvDq8ikWAM");
        assert_eq!(config.model, "eleven_multilingual_v2");
        assert_eq!(config.stability, 0.5);
        assert_eq!(config.similarity_boost, 0.75);
        assert_eq!(config.style, 0.0);
        assert!(config.use_speaker_boost);
    }

    #[test]
    fn test_voice_presets() {
        assert_eq!(Voice::rachel(), "21m00Tcm4TlvDq8ikWAM");
        assert_eq!(Voice::drew(), "29vD33N1CtxCmqQRPOHJ");
        assert_eq!(Voice::clyde(), "2EiwWnXFnvU5JabPnv8n");
    }

    #[test]
    fn test_voice_settings_clamping() {
        let mut tts = ElevenLabsTTS::new("test_key".to_string());

        // Test clamping
        tts.set_voice_settings(2.0, -0.5, 1.5);
        assert_eq!(tts.config.stability, 1.0);
        assert_eq!(tts.config.similarity_boost, 0.0);
        assert_eq!(tts.config.style, 1.0);
    }
}
