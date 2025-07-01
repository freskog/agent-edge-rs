// Removed unused imports
use crate::speech_producer::{SpeechChunk, SpeechEvent};
use futures_util::{SinkExt, StreamExt};
use log::{info, warn};
use serde::Deserialize;
use serde_json::{self, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::sync::broadcast;
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use url::Url;

const MAX_CHUNKS_PER_BATCH: usize = 12; // Maximum number of chunks to collect in one batch

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
            prompt: Some("Transcribe the following audio accurately.".to_string()),
            server_timeout: Duration::from_secs(30),
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

#[derive(Debug)]
struct TranscriptionStats {
    total_chunks: usize,
    total_batches: usize,
    max_batch_size: usize,
    avg_batch_size: f32,
    total_audio_duration_ms: f32,
    last_chunk_time: Option<Instant>,
    transcription_end_time: Option<Instant>,
    total_processing_time_ms: u128,
}

impl TranscriptionStats {
    fn new() -> Self {
        Self {
            total_chunks: 0,
            total_batches: 0,
            max_batch_size: 0,
            avg_batch_size: 0.0,
            total_audio_duration_ms: 0.0,
            last_chunk_time: None,
            transcription_end_time: None,
            total_processing_time_ms: 0,
        }
    }

    fn log_stats(&self) {
        info!("📊 Transcription Statistics:");
        info!("   Total chunks processed: {}", self.total_chunks);
        info!("   Number of batches sent: {}", self.total_batches);
        info!("   Maximum batch size: {} chunks", self.max_batch_size);
        info!("   Average batch size: {:.1} chunks", self.avg_batch_size);
        info!(
            "   Total audio duration: {:.1}ms",
            self.total_audio_duration_ms
        );
        info!(
            "   Total processing time: {}ms",
            self.total_processing_time_ms
        );

        if let (Some(last_chunk), Some(end_time)) =
            (self.last_chunk_time, self.transcription_end_time)
        {
            let time_to_transcript = end_time.duration_since(last_chunk).as_millis();
            info!(
                "   ⚡ Time from last chunk to transcript: {}ms",
                time_to_transcript
            );
        }
    }
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

    /// Collects available chunks from the receiver up to MAX_CHUNKS_PER_BATCH
    /// Returns (chunks, should_send_checkpoint, last_chunk_time)
    async fn collect_audio_chunks(
        &self,
        speech_receiver: &mut broadcast::Receiver<SpeechChunk>,
    ) -> (Vec<Vec<u8>>, bool, Option<Instant>) {
        let mut pcm_chunks = Vec::new();
        let mut should_send_checkpoint = false;
        let mut last_chunk_time = None;

        // Try to collect up to MAX_CHUNKS_PER_BATCH chunks
        while pcm_chunks.len() < MAX_CHUNKS_PER_BATCH {
            match speech_receiver.try_recv() {
                Ok(chunk) => {
                    last_chunk_time = Some(chunk.timestamp);
                    match chunk.speech_event {
                        SpeechEvent::StartedSpeaking | SpeechEvent::Speaking => {
                            if let Some(pcm_data) = self.samples_to_pcm(&chunk.samples_f32) {
                                pcm_chunks.push(pcm_data);
                            }
                        }
                        SpeechEvent::StoppedSpeaking => {
                            should_send_checkpoint = true;
                            break;
                        }
                    }
                }
                Err(broadcast::error::TryRecvError::Empty) => break,
                Err(broadcast::error::TryRecvError::Lagged(skipped)) => {
                    warn!("STT: Speech receiver lagged, skipped {} messages", skipped);
                    continue;
                }
                Err(broadcast::error::TryRecvError::Closed) => {
                    should_send_checkpoint = true;
                    break;
                }
            }
        }

        (pcm_chunks, should_send_checkpoint, last_chunk_time)
    }

    /// Transcribe streaming audio with initial context chunks
    /// This allows us to include recent audio chunks that were captured during wakeword detection
    pub async fn transcribe_stream_with_context(
        self: Arc<Self>,
        mut speech_receiver: broadcast::Receiver<SpeechChunk>,
        context_chunks: Vec<SpeechChunk>,
    ) -> Result<String, STTError> {
        let start_time = Instant::now();
        let stats = Arc::new(Mutex::new(TranscriptionStats::new()));

        // Add context chunks to total count and update last chunk time if any
        if let Ok(mut stats) = stats.lock() {
            stats.total_chunks += context_chunks.len();
            stats.total_audio_duration_ms += (context_chunks.len() * 1280) as f32 / 16.0; // 16kHz sample rate
            if let Some(last_context) = context_chunks.last() {
                stats.last_chunk_time = Some(last_context.timestamp);
            }
        }

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
        let (close_tx, mut close_rx) = mpsc::channel::<()>(1);
        let (final_tx, _final_rx) = mpsc::channel::<()>(1);

        // Clone self for the async block
        let self_clone = Arc::clone(&self);
        let stats_clone = Arc::clone(&stats);

        // Spawn a dedicated task for sending audio
        let _sender_handle = tokio::spawn(async move {
            let mut chunk_count = 0;
            let mut final_checkpoint_sent = false;

            // First, send all context chunks as a single message
            if !context_chunks.is_empty() {
                let mut context_buffer = Vec::new();
                for chunk in context_chunks {
                    if let Some(pcm_data) = self_clone.samples_to_pcm(&chunk.samples_f32) {
                        context_buffer.extend(pcm_data);
                    }
                }

                if !context_buffer.is_empty() {
                    log::debug!(
                        "STT: Sending context audio ({} bytes)",
                        context_buffer.len()
                    );
                    if write
                        .send(Message::Binary(context_buffer.into()))
                        .await
                        .is_err()
                    {
                        log::warn!("STT: Failed to send context audio");
                        return;
                    }
                    if let Ok(mut stats) = stats_clone.lock() {
                        stats.total_batches += 1;
                    }
                }
            }

            // Then continue with live stream
            loop {
                if final_checkpoint_sent {
                    match close_rx.try_recv() {
                        Ok(_) => {
                            log::info!("STT: Received close signal after final checkpoint");
                            break;
                        }
                        Err(mpsc::error::TryRecvError::Empty) => {
                            tokio::time::sleep(Duration::from_millis(100)).await;
                            continue;
                        }
                        Err(mpsc::error::TryRecvError::Disconnected) => {
                            log::info!("STT: Close channel disconnected");
                            break;
                        }
                    }
                }

                // Collect available chunks
                let (pcm_chunks, should_send_checkpoint, last_chunk_time) =
                    self_clone.collect_audio_chunks(&mut speech_receiver).await;

                // Update last chunk time if we got one
                if let Some(timestamp) = last_chunk_time {
                    if let Ok(mut stats) = stats_clone.lock() {
                        stats.last_chunk_time = Some(timestamp);
                    }
                }

                // Send collected chunks if any
                if !pcm_chunks.is_empty() {
                    let chunks_len = pcm_chunks.len();
                    chunk_count += chunks_len;
                    let mut combined = Vec::new();
                    for chunk in pcm_chunks {
                        combined.extend(chunk);
                    }

                    // Update stats
                    if let Ok(mut stats) = stats_clone.lock() {
                        stats.total_chunks += chunks_len;
                        stats.total_batches += 1;
                        stats.max_batch_size = stats.max_batch_size.max(chunks_len);
                        stats.avg_batch_size =
                            (stats.total_chunks as f32) / (stats.total_batches as f32);
                        stats.total_audio_duration_ms += (chunks_len * 1280) as f32 / 16.0;
                    }

                    log::debug!(
                        "STT: Sending audio batch of {} chunks ({} bytes)",
                        chunks_len,
                        combined.len()
                    );

                    if write.send(Message::Binary(combined.into())).await.is_err() {
                        log::warn!("STT: Failed to send audio batch");
                        break;
                    }
                }

                // Handle checkpoint if needed
                if should_send_checkpoint {
                    log::info!("STT: Sending final checkpoint");
                    let final_checkpoint = json!({"checkpoint_id": "final"});
                    let checkpoint_msg = serde_json::to_string(&final_checkpoint).unwrap();

                    if let Err(e) = write.send(Message::Text(checkpoint_msg.into())).await {
                        log::warn!("STT: Failed to send final checkpoint: {}", e);
                        break;
                    } else {
                        log::info!(
                            "STT: Final checkpoint sent successfully, waiting for acknowledgment"
                        );
                        final_checkpoint_sent = true;
                    }
                }

                if close_rx.try_recv().is_ok() {
                    log::info!("STT: Received close signal");
                    break;
                }

                // Small delay to prevent tight polling
                tokio::time::sleep(Duration::from_millis(10)).await;
            }

            log::info!(
                "STT: Closing speech sender, processed {} chunks total",
                chunk_count
            );
            let _ = write.close().await;
        });

        // Get timeout from config before moving self
        let server_timeout = self.config.server_timeout;

        // This main task handles receiving messages and waits for final checkpoint
        let mut final_transcript = String::new();
        let mut message_count = 0;
        let mut received_final_checkpoint = false;
        let mut last_message_time = Instant::now();
        let mut _segments: HashMap<u32, String> = HashMap::new();

        log::info!("STT: Starting to listen for server responses");

        // Process messages until we get final checkpoint or timeout
        while let Ok(Some(msg_result)) = tokio::time::timeout(server_timeout, read.next()).await {
            message_count += 1;

            // Check for timeout since last message
            if last_message_time.elapsed() > Duration::from_secs(10) {
                log::warn!("STT: No messages received for 10 seconds");
                return Err(STTError::Streaming(
                    "No response for 10 seconds".to_string(),
                ));
            }

            // Update last message time
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
                                let _ = final_tx.send(()).await;
                                break;
                            }

                            // Update segments and transcript
                            if let Ok(response) =
                                serde_json::from_str::<StreamingResponse>(&text_str)
                            {
                                if let Some(new_segments) = response.segments {
                                    for segment in new_segments {
                                        _segments.insert(segment.id, segment.text);
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

        // Update final stats before returning
        if let Ok(mut stats) = stats.lock() {
            stats.transcription_end_time = Some(Instant::now());
            stats.total_processing_time_ms = start_time.elapsed().as_millis();
            stats.log_stats();
        }

        Ok(final_transcript)
    }

    /// Convert f32 samples to PCM 16-bit little-endian format
    fn samples_to_pcm(&self, samples: &[f32]) -> Option<Vec<u8>> {
        let mut pcm_data = Vec::with_capacity(samples.len() * 2);
        for &sample in samples {
            // Convert f32 [-1.0, 1.0] to i16 [-32768, 32767]
            let value = (sample * 32767.0) as i16;
            pcm_data.extend_from_slice(&value.to_le_bytes());
        }
        Some(pcm_data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio_capture::AudioChunk;
    use std::time::Instant;

    // Helper function to create a test audio chunk
    fn create_test_chunk() -> AudioChunk {
        AudioChunk {
            samples: vec![0i16; 1280],
            timestamp: Instant::now(),
        }
    }

    #[test]
    fn test_audio_chunk_creation() {
        let chunk = create_test_chunk();
        assert_eq!(chunk.samples.len(), 1280);
        assert!(chunk.timestamp.elapsed().as_millis() < 100);
    }

    #[tokio::test]
    async fn test_audio_chunk_broadcast() {
        let chunk = create_test_chunk();
        let (tx, mut rx) = broadcast::channel::<AudioChunk>(10);

        // Send the chunk
        tx.send(chunk.clone()).unwrap();

        // Receive the chunk
        let received = rx.recv().await.unwrap();
        assert_eq!(received.samples.len(), chunk.samples.len());
        assert_eq!(received.timestamp, chunk.timestamp);
    }

    #[tokio::test]
    async fn test_audio_chunk_multiple_receivers() {
        let chunk = create_test_chunk();
        let (tx, mut rx1) = broadcast::channel::<AudioChunk>(10);
        let mut rx2 = tx.subscribe();

        // Send the chunk
        tx.send(chunk.clone()).unwrap();

        // Both receivers should get the chunk
        let received1 = rx1.recv().await.unwrap();
        let received2 = rx2.recv().await.unwrap();

        assert_eq!(received1.samples.len(), chunk.samples.len());
        assert_eq!(received2.samples.len(), chunk.samples.len());
    }

    #[tokio::test]
    async fn test_config_defaults() {
        let config = STTConfig::default();
        assert_eq!(config.language, None);
        assert_eq!(config.temperature, Some(0.0));
        assert_eq!(
            config.prompt,
            Some("Transcribe the following audio accurately.".to_string())
        );
        assert_eq!(config.server_timeout, Duration::from_secs(30));
    }

    #[tokio::test]
    async fn test_stt_creation() {
        let stt = FireworksSTT::new("test_key".to_string());
        assert_eq!(stt.api_key, "test_key");
        assert_eq!(stt.config.server_timeout, Duration::from_secs(30));
    }

    #[tokio::test]
    async fn test_stt_with_custom_config() {
        let mut config = STTConfig::default();
        config.language = Some("en".to_string());
        config.temperature = Some(0.5);
        config.server_timeout = Duration::from_secs(50);

        let stt = FireworksSTT::with_config("test_key".to_string(), config);
        assert_eq!(stt.api_key, "test_key");
        assert_eq!(stt.config.language, Some("en".to_string()));
        assert_eq!(stt.config.temperature, Some(0.5));
        assert_eq!(stt.config.server_timeout, Duration::from_secs(50));
    }

    #[tokio::test]
    async fn test_samples_to_pcm() {
        let stt = FireworksSTT::new("test_key".to_string());
        let samples = vec![0.0f32, 0.5f32, -0.5f32, 1.0f32];

        let result = stt.samples_to_pcm(&samples);
        assert!(result.is_some());

        let pcm_data = result.unwrap();
        assert_eq!(pcm_data.len(), samples.len() * 2); // 2 bytes per sample (16-bit)
    }
}
