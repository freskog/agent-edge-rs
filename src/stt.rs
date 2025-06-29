use crate::AudioChunk;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{self, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
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
    pub server_timeout: Duration,
}

impl Default for STTConfig {
    fn default() -> Self {
        Self {
            language: None,
            temperature: Some(0.0),
            prompt: None, // No biasing prompt - let it transcribe naturally
            server_timeout: Duration::from_millis(10000), // 10 seconds default timeout
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

        // Add API key as a query parameter
        url.query_pairs_mut()
            .append_pair("Authorization", &self.api_key);

        // Connect to WebSocket using the URL as a string
        let (ws_stream, _) = connect_async(url.as_str()).await?;
        let (mut write, mut read) = ws_stream.split();

        // Create channels for signaling
        let (close_tx, mut close_rx) = tokio::sync::mpsc::channel::<()>(1);
        let (final_tx, mut final_rx) = tokio::sync::mpsc::channel::<()>(1);

        // Spawn a dedicated task for sending audio
        let sender_handle = tokio::spawn(async move {
            let mut chunk_count = 0;
            let mut last_send_time = Instant::now();
            let chunk_interval = Duration::from_millis(80); // 80ms per chunk

            loop {
                match audio_receiver.recv().await {
                    Ok(audio_chunk) => {
                        chunk_count += 1;
                        let pcm_data = self.samples_to_pcm(&audio_chunk.samples_f32).unwrap(); // Infallible
                        let samples_count = audio_chunk.samples_f32.len();
                        let duration_ms = (samples_count as f32 / 16000.0) * 1000.0;

                        log::debug!(
                            "STT: Sending audio chunk {} ({} samples = {:.1}ms = {} bytes)",
                            chunk_count,
                            samples_count,
                            duration_ms,
                            pcm_data.len()
                        );

                        // Maintain real-time pacing
                        let elapsed = last_send_time.elapsed();
                        if elapsed < chunk_interval {
                            tokio::time::sleep(chunk_interval - elapsed).await;
                        }
                        last_send_time = Instant::now();

                        if write.send(Message::Binary(pcm_data.into())).await.is_err() {
                            log::warn!("STT: Failed to send audio chunk {}", chunk_count);
                            break; // Connection closed
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        log::warn!("STT: Audio receiver lagged, skipped {} messages", skipped);
                        continue;
                    }
                    Err(_) => {
                        log::info!(
                            "STT: Audio channel closed, sent {} chunks total",
                            chunk_count
                        );
                        break; // Channel closed
                    }
                }

                // Check if we should close
                if close_rx.try_recv().is_ok() {
                    log::info!("STT: Received close signal");
                    break;
                }
            }

            // Send final checkpoint message
            let final_checkpoint = json!({"checkpoint_id": "final"});
            let checkpoint_msg = serde_json::to_string(&final_checkpoint).unwrap();
            log::info!("STT: Sending final checkpoint");

            // Send as text message, not binary
            if let Err(e) = write.send(Message::Text(checkpoint_msg.into())).await {
                log::warn!("STT: Failed to send final checkpoint: {}", e);
            }

            // Wait for final checkpoint acknowledgment or timeout
            let timeout = Duration::from_secs(3);
            match tokio::time::timeout(timeout, final_rx.recv()).await {
                Ok(Some(_)) => {
                    log::info!("STT: Received final checkpoint acknowledgment");
                }
                Ok(None) => {
                    log::warn!("STT: Final checkpoint channel closed");
                }
                Err(_) => {
                    log::warn!("STT: Timeout waiting for final checkpoint acknowledgment");
                }
            }

            log::info!(
                "STT: Closing audio sender, sent {} chunks total",
                chunk_count
            );
            let _ = write.close().await; // Close the write half when done
        });

        // This main task handles receiving messages and waits for final checkpoint
        let mut final_transcript = String::new();
        let mut message_count = 0;
        let mut received_final_checkpoint = false;
        let mut last_message_time = Instant::now();
        let mut segments: HashMap<u32, String> = HashMap::new();

        log::info!("STT: Starting to listen for server responses");

        // Process messages until we get final checkpoint or timeout
        let server_timeout = Duration::from_millis(10000); // 10 seconds to allow for longer processing
        while let Ok(Some(msg_result)) = tokio::time::timeout(server_timeout, read.next()).await {
            message_count += 1;
            last_message_time = Instant::now();

            match msg_result {
                Ok(Message::Text(text)) => {
                    let text_str = text.to_string();
                    log::debug!("STT: Received text message {}: {}", message_count, text_str);

                    // Parse message as JSON
                    match serde_json::from_str::<serde_json::Value>(&text_str) {
                        Ok(data) => {
                            // Check for final checkpoint
                            if data.get("checkpoint_id").and_then(|v| v.as_str()) == Some("final") {
                                log::info!(
                                    "STT: Received final checkpoint acknowledgment from server"
                                );
                                received_final_checkpoint = true;

                                // Signal that we got the final checkpoint
                                let _ = final_tx.send(()).await;
                                break;
                            }

                            // Update segments and transcript
                            if let Ok(response) =
                                serde_json::from_str::<StreamingResponse>(&text_str)
                            {
                                if let Some(new_segments) = response.segments {
                                    for segment in new_segments {
                                        segments.insert(segment.id, segment.text);
                                    }
                                }
                                if let Some(text) = response.text {
                                    if !text.is_empty() {
                                        log::info!("STT: Updated transcript: '{}'", text);
                                        final_transcript = text;
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            log::warn!("STT: Failed to parse message as JSON: {}", e);
                            // Continue processing - don't fail on parse errors
                        }
                    }
                }
                Ok(Message::Binary(data)) => {
                    log::debug!(
                        "STT: Received binary message {} ({} bytes)",
                        message_count,
                        data.len()
                    );
                }
                Ok(Message::Close(frame)) => {
                    log::info!("STT: Server closed connection: {:?}", frame);
                    break;
                }
                Err(e) => {
                    log::error!("STT: WebSocket error: {}", e);
                    break;
                }
                _ => {
                    log::debug!("STT: Received other message type {}", message_count);
                }
            }

            // Check for timeout since last message
            if last_message_time.elapsed() > Duration::from_secs(3) {
                log::warn!("STT: No messages received for 3 seconds");
                return Err(STTError::Streaming("No response for 3 seconds".to_string()));
            }
        }

        // Signal the sender to close
        let _ = close_tx.send(()).await;

        // Check if we got the final checkpoint
        if !received_final_checkpoint {
            log::warn!("STT: Did not receive final checkpoint acknowledgment from server");
            return Err(STTError::Streaming(
                "No final checkpoint acknowledgment".to_string(),
            ));
        }

        log::info!(
            "STT: Transcription complete - received {} messages, final transcript: '{}'",
            message_count,
            final_transcript
        );

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
    use std::time::{Duration, Instant};
    use tokio::sync::broadcast;

    // Helper function to create a test audio chunk
    fn create_test_chunk() -> AudioChunk {
        AudioChunk {
            samples_i16: vec![0i16; 1280],
            samples_f32: vec![0.0f32; 1280],
            timestamp: Instant::now(),
            should_process: true,
        }
    }

    #[tokio::test]
    async fn test_config_defaults() {
        let config = STTConfig::default();
        assert_eq!(config.language, None);
        assert_eq!(config.temperature, Some(0.0));
        assert_eq!(config.prompt, None);
        assert_eq!(config.server_timeout, Duration::from_millis(10000));
    }

    #[tokio::test]
    async fn test_stt_creation() {
        let stt = FireworksSTT::new("test_key".to_string());
        assert_eq!(stt.api_key, "test_key");
        assert_eq!(stt.config.server_timeout, Duration::from_millis(10000));
    }

    #[tokio::test]
    async fn test_stt_with_custom_config() {
        let mut config = STTConfig::default();
        config.language = Some("en".to_string());
        config.temperature = Some(0.5);
        config.server_timeout = Duration::from_millis(5000);

        let stt = FireworksSTT::with_config("test_key".to_string(), config);
        assert_eq!(stt.api_key, "test_key");
        assert_eq!(stt.config.language, Some("en".to_string()));
        assert_eq!(stt.config.temperature, Some(0.5));
        assert_eq!(stt.config.server_timeout, Duration::from_millis(5000));
    }

    #[tokio::test]
    async fn test_samples_to_pcm() {
        let stt = FireworksSTT::new("test_key".to_string());
        let samples = vec![0.0f32, 0.5f32, -0.5f32, 1.0f32];

        let result = stt.samples_to_pcm(&samples);
        assert!(result.is_ok());

        let pcm_data = result.unwrap();
        assert_eq!(pcm_data.len(), samples.len() * 2); // 2 bytes per sample (16-bit)
    }

    #[tokio::test]
    async fn test_audio_chunk_creation() {
        let chunk = create_test_chunk();
        assert_eq!(chunk.samples_i16.len(), 1280);
        assert_eq!(chunk.samples_f32.len(), 1280);
        assert!(chunk.should_process);
    }

    #[tokio::test]
    async fn test_broadcast_channel_basic() {
        let (tx, mut rx) = broadcast::channel::<AudioChunk>(10);

        // Send a test chunk
        let chunk = create_test_chunk();
        tx.send(chunk.clone()).unwrap();

        // Receive the chunk
        let received = rx.recv().await.unwrap();
        assert_eq!(received.samples_i16.len(), chunk.samples_i16.len());
        assert_eq!(received.should_process, chunk.should_process);
    }

    #[tokio::test]
    async fn test_broadcast_channel_dropped_sender() {
        let (tx, mut rx) = broadcast::channel::<AudioChunk>(10);

        // Drop the sender
        drop(tx);

        // Try to receive - should get an error
        let result = rx.recv().await;
        assert!(result.is_err());
    }
}
