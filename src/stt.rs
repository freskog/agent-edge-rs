use reqwest::{Client, multipart};
use std::time::Duration;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum STTError {
    #[error("HTTP request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("API error: {status} - {message}")]
    ApiError { status: u16, message: String },
    #[error("Audio format error: {0}")]
    AudioFormat(String),
    #[error("Response parsing error: {0}")]
    ParseError(String),
}

#[derive(Debug, Clone)]
pub struct STTConfig {
    pub model: String,
    pub language: Option<String>,
    pub temperature: Option<f32>,
    pub response_format: String,
}

impl Default for STTConfig {
    fn default() -> Self {
        Self {
            model: "whisper-v3".to_string(),
            language: None,         // Auto-detect
            temperature: Some(0.0), // Deterministic
            response_format: "json".to_string(),
        }
    }
}

#[derive(Debug)]
pub struct STTResponse {
    pub text: String,
    pub language: Option<String>,
    pub duration: Option<f32>,
}

pub struct FireworksSTT {
    client: Client,
    api_key: String,
    base_url: String,
    config: STTConfig,
}

impl FireworksSTT {
    pub fn new(api_key: String) -> Self {
        Self::with_config(api_key, STTConfig::default())
    }

    pub fn with_config(api_key: String, config: STTConfig) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            client,
            api_key,
            base_url: "https://api.fireworks.ai/inference/v1".to_string(),
            config,
        }
    }

    /// Transcribe audio from raw bytes (WAV format)
    pub async fn transcribe_bytes(&self, audio_data: &[u8]) -> Result<STTResponse, STTError> {
        self.transcribe_with_filename(audio_data, "audio.wav").await
    }

    /// Transcribe audio with specific filename for format detection
    pub async fn transcribe_with_filename(
        &self,
        audio_data: &[u8],
        filename: &str,
    ) -> Result<STTResponse, STTError> {
        let url = format!("{}/audio/transcriptions", self.base_url);

        // Build multipart form
        let mut form = multipart::Form::new()
            .text("model", self.config.model.clone())
            .text("response_format", self.config.response_format.clone())
            .part(
                "file",
                multipart::Part::bytes(audio_data.to_vec())
                    .file_name(filename.to_string())
                    .mime_str("audio/wav")
                    .map_err(|e| STTError::AudioFormat(format!("Invalid MIME type: {}", e)))?,
            );

        // Add optional parameters
        if let Some(language) = &self.config.language {
            form = form.text("language", language.clone());
        }

        if let Some(temperature) = self.config.temperature {
            form = form.text("temperature", temperature.to_string());
        }

        // Make the request
        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .multipart(form)
            .send()
            .await?;

        let status = response.status();

        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(STTError::ApiError {
                status: status.as_u16(),
                message: error_text,
            });
        }

        // Parse response
        let response_text = response.text().await?;
        self.parse_response(&response_text)
    }

    /// Parse the JSON response from Fireworks API
    fn parse_response(&self, response_text: &str) -> Result<STTResponse, STTError> {
        let json: serde_json::Value = serde_json::from_str(response_text)
            .map_err(|e| STTError::ParseError(format!("Invalid JSON: {}", e)))?;

        let text = json["text"]
            .as_str()
            .ok_or_else(|| STTError::ParseError("Missing 'text' field".to_string()))?
            .to_string();

        let language = json["language"].as_str().map(|s| s.to_string());
        let duration = json["duration"].as_f64().map(|d| d as f32);

        Ok(STTResponse {
            text,
            language,
            duration,
        })
    }

    /// Convert audio from f32 samples to WAV bytes for transmission
    pub fn samples_to_wav(&self, samples: &[f32], sample_rate: u32) -> Result<Vec<u8>, STTError> {
        use std::io::Cursor;

        let mut cursor = Cursor::new(Vec::new());

        // Write WAV header
        self.write_wav_header(&mut cursor, samples.len() as u32, sample_rate)
            .map_err(|e| STTError::AudioFormat(format!("WAV header error: {}", e)))?;

        // Convert f32 samples to i16 and write
        for &sample in samples {
            let sample_i16 = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
            cursor
                .get_mut()
                .extend_from_slice(&sample_i16.to_le_bytes());
        }

        Ok(cursor.into_inner())
    }

    /// Write WAV file header
    fn write_wav_header(
        &self,
        cursor: &mut std::io::Cursor<Vec<u8>>,
        num_samples: u32,
        sample_rate: u32,
    ) -> Result<(), Box<dyn std::error::Error>> {
        use std::io::Write;

        let byte_rate = sample_rate * 2; // 16-bit mono
        let data_size = num_samples * 2;
        let file_size = 36 + data_size;

        // RIFF chunk
        cursor.write_all(b"RIFF")?;
        cursor.write_all(&file_size.to_le_bytes())?;
        cursor.write_all(b"WAVE")?;

        // fmt chunk
        cursor.write_all(b"fmt ")?;
        cursor.write_all(&16u32.to_le_bytes())?; // chunk size
        cursor.write_all(&1u16.to_le_bytes())?; // audio format (PCM)
        cursor.write_all(&1u16.to_le_bytes())?; // num channels (mono)
        cursor.write_all(&sample_rate.to_le_bytes())?;
        cursor.write_all(&byte_rate.to_le_bytes())?;
        cursor.write_all(&2u16.to_le_bytes())?; // block align
        cursor.write_all(&16u16.to_le_bytes())?; // bits per sample

        // data chunk
        cursor.write_all(b"data")?;
        cursor.write_all(&data_size.to_le_bytes())?;

        Ok(())
    }

    /// Convenience method to transcribe f32 audio samples directly
    pub async fn transcribe_samples(
        &self,
        samples: &[f32],
        sample_rate: u32,
    ) -> Result<STTResponse, STTError> {
        let wav_data = self.samples_to_wav(samples, sample_rate)?;
        self.transcribe_bytes(&wav_data).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wav_conversion() {
        let stt = FireworksSTT::new("test_key".to_string());

        // Generate a simple sine wave
        let sample_rate = 16000;
        let duration = 1.0; // 1 second
        let frequency = 440.0; // A4 note

        let samples: Vec<f32> = (0..((sample_rate as f32 * duration) as usize))
            .map(|i| {
                let t = i as f32 / sample_rate as f32;
                (2.0 * std::f32::consts::PI * frequency * t).sin() * 0.5
            })
            .collect();

        let wav_data = stt.samples_to_wav(&samples, sample_rate).unwrap();

        // Check WAV header
        assert_eq!(&wav_data[0..4], b"RIFF");
        assert_eq!(&wav_data[8..12], b"WAVE");
        assert_eq!(&wav_data[12..16], b"fmt ");

        // Check that we have the expected amount of data
        assert!(wav_data.len() > 44); // WAV header is 44 bytes
    }

    #[test]
    fn test_config_defaults() {
        let config = STTConfig::default();
        assert_eq!(config.model, "whisper-v3");
        assert_eq!(config.response_format, "json");
        assert_eq!(config.temperature, Some(0.0));
        assert!(config.language.is_none());
    }
}
