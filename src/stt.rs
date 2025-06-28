use crate::AudioChunk;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{self, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::sync::broadcast;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use url::Url;

#[derive(Error, Debug)]
pub enum STTError {
    #[error("WebSocket connection failed: {0}")]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),
    #[error("URL parsing error: {0}")]
    UrlParse(#[from] url::ParseError),
    #[error("API error: {message}")]
    ApiError { message: String },
    #[error("Audio format error: {0}")]
    AudioFormat(String),
    #[error("Response parsing error: {0}")]
    ParseError(String),
    #[error("Streaming error: {0}")]
    Streaming(String),
}

#[derive(Debug, Clone)]
pub struct STTConfig {
    pub language: Option<String>,
    pub temperature: Option<f32>,
    pub prompt: Option<String>,
}

impl Default for STTConfig {
    fn default() -> Self {
        Self {
            language: None,
            temperature: Some(0.0),
            prompt: None, // No biasing prompt - let it transcribe naturally
        }
    }
}

#[derive(Debug, Clone)]
pub struct STTResponse {
    pub text: String,
    pub language: Option<String>,
    pub segments: HashMap<u32, String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct StreamingResponse {
    task: Option<String>,
    language: Option<String>,
    text: Option<String>,
    segments: Option<Vec<StreamingSegment>>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct StreamingSegment {
    id: u32,
    text: String,
}

pub struct FireworksSTT {
    api_key: String,
    config: STTConfig,
}

impl FireworksSTT {
    pub fn new(api_key: String) -> Self {
        Self::with_config(api_key, STTConfig::default())
    }

    pub fn with_config(api_key: String, config: STTConfig) -> Self {
        Self { api_key, config }
    }

    /// Establishes a WebSocket connection, streams all audio from a receiver,
    /// and returns the final transcript received from the server.
    pub async fn transcribe_stream(
        self: Arc<Self>,
        mut audio_receiver: broadcast::Receiver<AudioChunk>,
    ) -> Result<String, STTError> {
        // Create WebSocket URL with query parameters
        let mut url = Url::parse(
            "wss://audio-streaming.us-virginia-1.direct.fireworks.ai/v1/audio/transcriptions/streaming",
        )?;

        // Add query parameters
        if let Some(language) = &self.config.language {
            url.query_pairs_mut().append_pair("language", language);
        }

        if let Some(temperature) = self.config.temperature {
            url.query_pairs_mut()
                .append_pair("temperature", &temperature.to_string());
        }

        if let Some(prompt) = &self.config.prompt {
            url.query_pairs_mut().append_pair("prompt", prompt);
        }

        // Set response format to verbose_json for streaming
        url.query_pairs_mut()
            .append_pair("response_format", "verbose_json");

        // Add API key as a query parameter (this was working before)
        url.query_pairs_mut()
            .append_pair("Authorization", &self.api_key);

        // Connect to WebSocket using the URL as a string
        let (ws_stream, _) = connect_async(url.as_str()).await?;
        let (mut write, mut read) = ws_stream.split();

        // Spawn a dedicated task for sending audio.
        let sender_handle = tokio::spawn(async move {
            let mut chunk_count = 0;
            
            loop {
                match audio_receiver.recv().await {
                    Ok(audio_chunk) => {
                        chunk_count += 1;
                        let pcm_data = self.samples_to_pcm(&audio_chunk.samples_f32).unwrap(); // Infallible
                        let samples_count = audio_chunk.samples_f32.len();
                        let duration_ms = (samples_count as f32 / 16000.0) * 1000.0;
                        
                        log::debug!("STT: Sending audio chunk {} ({} samples = {:.1}ms = {} bytes)", 
                                   chunk_count, samples_count, duration_ms, pcm_data.len());
                        
                        if write.send(Message::Binary(pcm_data.into())).await.is_err() {
                            log::warn!("STT: Failed to send audio chunk {}", chunk_count);
                            break; // Connection closed
                        }
                        // No artificial sleep - we're getting real-time audio from microphone
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        log::warn!("STT: Audio receiver lagged, skipped {} messages", skipped);
                        continue;
                    }
                    Err(_) => {
                        log::info!("STT: Audio channel closed, sent {} chunks total", chunk_count);
                        break; // Channel closed
                    }
                }
            }
            
            // Send final checkpoint message like Python example
            let final_checkpoint = json!({"checkpoint_id": "final"});
            let checkpoint_msg = serde_json::to_string(&final_checkpoint).unwrap();
            log::info!("STT: Sending final checkpoint");
            if let Err(e) = write.send(Message::Text(checkpoint_msg.into())).await {
                log::warn!("STT: Failed to send final checkpoint: {}", e);
            }
            
            log::info!("STT: Closing audio sender, sent {} chunks total", chunk_count);
            let _ = write.close().await; // Close the write half when done
        });

        // This main task handles receiving messages and waits for final checkpoint
        let mut final_transcript = String::new();
        let mut message_count = 0;
        let mut received_final_checkpoint = false;

        log::info!("STT: Starting to listen for server responses");
        
        // First phase: Process messages until we get final checkpoint or timeout
        let server_timeout = Duration::from_millis(10000); // 10 seconds to allow for longer processing
        while let Ok(Some(msg_result)) = tokio::time::timeout(server_timeout, read.next()).await {
            message_count += 1;
            match msg_result {
                Ok(Message::Text(text)) => {
                    log::debug!("STT: Received text message {}: {}", message_count, text);
                    
                    // Check for final checkpoint response
                    if let Ok(checkpoint_response) = serde_json::from_str::<serde_json::Value>(&text.to_string()) {
                        if checkpoint_response.get("checkpoint_id").and_then(|v| v.as_str()) == Some("final") {
                            log::info!("STT: Received final checkpoint acknowledgment from server");
                            received_final_checkpoint = true;
                            break; // Exit the loop when we get final checkpoint
                        }
                    }
                    
                    if let Ok(response) = serde_json::from_str::<StreamingResponse>(&text.to_string()) {
                        log::debug!("STT: Parsed response - text: {:?}", response.text);
                        if let Some(text) = response.text {
                            if !text.is_empty() {
                                log::info!("STT: Updated transcript: '{}'", text);
                                final_transcript = text;
                            }
                        }
                    } else {
                        log::debug!("STT: Non-transcription message: {}", text);
                    }
                }
                Ok(Message::Binary(data)) => {
                    log::debug!("STT: Received binary message {} ({} bytes)", message_count, data.len());
                }
                Ok(Message::Close(frame)) => {
                    log::info!("STT: Server closed connection: {:?}", frame);
                    break; // Server closed connection
                }
                Err(e) => {
                    log::error!("STT: WebSocket error: {}", e);
                    break; // WebSocket error
                }
                _ => {
                    log::debug!("STT: Received other message type {}", message_count);
                }
            }
        }

        // Second phase: Wait for any final transcription updates after checkpoint
        if received_final_checkpoint {
            log::info!("STT: Final checkpoint received, waiting for final transcription...");
            let final_timeout = Duration::from_millis(3000); // 3 seconds for final processing
            while let Ok(Some(msg_result)) = tokio::time::timeout(final_timeout, read.next()).await {
                match msg_result {
                    Ok(Message::Text(text)) => {
                        log::debug!("STT: Post-checkpoint message: {}", text);
                        if let Ok(response) = serde_json::from_str::<StreamingResponse>(&text.to_string()) {
                            if let Some(text) = response.text {
                                if !text.is_empty() {
                                    log::info!("STT: Final transcript update: '{}'", text);
                                    final_transcript = text;
                                }
                            }
                        }
                    }
                    Ok(Message::Close(_)) => {
                        log::info!("STT: Server closed connection after final checkpoint");
                        break;
                    }
                    Err(e) => {
                        log::error!("STT: Error after final checkpoint: {}", e);
                        break;
                    }
                    _ => {}
                }
            }
        } else {
            log::warn!("STT: Did not receive final checkpoint acknowledgment from server");
        }

        log::info!("STT: Transcription complete - received {} messages, final transcript: '{}'", message_count, final_transcript);

        // Ensure the sender task is cleaned up
        sender_handle.abort();

        Ok(final_transcript)
    }

    /// Convert f32 samples to PCM 16-bit little-endian format
    fn samples_to_pcm(&self, samples: &[f32]) -> Result<Vec<u8>, STTError> {
        let mut pcm_data = Vec::with_capacity(samples.len() * 2);

        for &sample in samples {
            let sample_i16 = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
            pcm_data.extend_from_slice(&sample_i16.to_le_bytes());
        }

        Ok(pcm_data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pcm_conversion() {
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

        let pcm_data = stt.samples_to_pcm(&samples).unwrap();

        // Check that we have the expected amount of data (16-bit = 2 bytes per sample)
        assert_eq!(pcm_data.len(), samples.len() * 2);

        // Check that the data is little-endian
        assert_eq!(pcm_data[0], 0); // First sample should be close to 0
    }

    #[test]
    fn test_config_defaults() {
        let config = STTConfig::default();
        assert_eq!(config.language, None);
        assert_eq!(config.temperature, Some(0.0));
        assert!(config.prompt.is_some());
    }
}


