// Removed unused imports
use crate::speech_producer::{SpeechChunk, SpeechEvent};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{self, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::sync::broadcast;
use tokio_tungstenite::{connect_async, tungstenite::Message};
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
            language: Some("en".to_string()),
            temperature: Some(0.0),
            prompt: Some(
                "The user will say 'Hey Mycroft' followed by a question or command.".to_string(),
            ),
            server_timeout: Duration::from_millis(30000),
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

    /// Establishes a WebSocket connection, streams all speech from a receiver,
    /// and returns the final transcript received from the server.
    pub async fn transcribe_stream(
        self: Arc<Self>,
        mut speech_receiver: broadcast::Receiver<SpeechChunk>,
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

        // Clone self for the async block
        let self_clone = Arc::clone(&self);

        // Spawn a dedicated task for sending audio
        let sender_handle = tokio::spawn(async move {
            let mut chunk_count = 0;
            let mut final_checkpoint_sent = false;

            loop {
                // If we've sent the final checkpoint, just wait for close signal or channel close
                if final_checkpoint_sent {
                    match close_rx.try_recv() {
                        Ok(_) => {
                            log::info!("STT: Received close signal after final checkpoint");
                            break;
                        }
                        Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                            // No close signal yet, sleep a bit and check again
                            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                            continue;
                        }
                        Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                            log::info!("STT: Close channel disconnected");
                            break;
                        }
                    }
                }

                match speech_receiver.recv().await {
                    Ok(speech_chunk) => {
                        // Handle different speech events
                        match speech_chunk.speech_event {
                            SpeechEvent::StartedSpeaking | SpeechEvent::Speaking => {
                                chunk_count += 1;
                                let pcm_data = self_clone
                                    .samples_to_pcm(&speech_chunk.samples_f32)
                                    .unwrap(); // Infallible
                                let samples_count = speech_chunk.samples_f32.len();
                                let duration_ms = (samples_count as f32 / 16000.0) * 1000.0;

                                log::debug!(
                                    "STT: Sending speech chunk {} ({} samples = {:.1}ms = {} bytes)",
                                    chunk_count,
                                    samples_count,
                                    duration_ms,
                                    pcm_data.len()
                                );

                                // Send immediately without artificial pacing delays

                                if write.send(Message::Binary(pcm_data.into())).await.is_err() {
                                    log::warn!("STT: Failed to send speech chunk {}", chunk_count);
                                    break; // Connection closed
                                }
                            }
                            SpeechEvent::StoppedSpeaking => {
                                let stopped_speaking_time = Instant::now();
                                log::info!(
                                    "STT: Received StoppedSpeaking event at {:?}, waiting briefly for potential continuation...",
                                    stopped_speaking_time
                                );

                                // Wait a brief moment to see if speech resumes (natural pause handling)
                                tokio::time::sleep(Duration::from_millis(200)).await;

                                // Check if we received any new speech chunks during the pause
                                let mut should_send_checkpoint = true;
                                while let Ok(new_chunk) = speech_receiver.try_recv() {
                                    match new_chunk.speech_event {
                                        SpeechEvent::StartedSpeaking | SpeechEvent::Speaking => {
                                            // Speech resumed, continue processing instead of ending
                                            chunk_count += 1;
                                            let pcm_data = self_clone
                                                .samples_to_pcm(&new_chunk.samples_f32)
                                                .unwrap();
                                            let samples_count = new_chunk.samples_f32.len();
                                            let duration_ms =
                                                (samples_count as f32 / 16000.0) * 1000.0;

                                            log::debug!(
                                                "STT: Speech resumed! Sending chunk {} ({} samples = {:.1}ms = {} bytes)",
                                                chunk_count,
                                                samples_count,
                                                duration_ms,
                                                pcm_data.len()
                                            );

                                            if write
                                                .send(Message::Binary(pcm_data.into()))
                                                .await
                                                .is_err()
                                            {
                                                log::warn!(
                                                    "STT: Failed to send resumed speech chunk {}",
                                                    chunk_count
                                                );
                                                break;
                                            }
                                            should_send_checkpoint = false;
                                        }
                                        SpeechEvent::StoppedSpeaking => {
                                            // Another stop event, ignore it
                                            continue;
                                        }
                                    }
                                }

                                if should_send_checkpoint {
                                    let checkpoint_send_time = Instant::now();
                                    log::info!("STT: No speech resumption detected, sending final checkpoint at {:?}", checkpoint_send_time);

                                    // Send final checkpoint message
                                    let final_checkpoint = json!({"checkpoint_id": "final"});
                                    let checkpoint_msg =
                                        serde_json::to_string(&final_checkpoint).unwrap();

                                    if let Err(e) =
                                        write.send(Message::Text(checkpoint_msg.into())).await
                                    {
                                        log::warn!("STT: Failed to send final checkpoint: {}", e);
                                        break; // Break if we can't send the checkpoint
                                    } else {
                                        log::info!("STT: Final checkpoint sent successfully at {:?}, waiting for acknowledgment", checkpoint_send_time);
                                        final_checkpoint_sent = true;
                                    }
                                } else {
                                    log::info!(
                                        "STT: Speech resumed after pause, continuing transcription"
                                    );
                                }

                                // Continue the loop
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        log::warn!("STT: Speech receiver lagged, skipped {} messages", skipped);
                        continue;
                    }
                    Err(_) => {
                        if !final_checkpoint_sent {
                            log::info!(
                                "STT: Speech channel closed, sent {} chunks total",
                                chunk_count
                            );

                            // Send final checkpoint if channel closed without StoppedSpeaking
                            let final_checkpoint = json!({"checkpoint_id": "final"});
                            let checkpoint_msg = serde_json::to_string(&final_checkpoint).unwrap();

                            if let Err(e) = write.send(Message::Text(checkpoint_msg.into())).await {
                                log::warn!(
                                    "STT: Failed to send final checkpoint on channel close: {}",
                                    e
                                );
                            } else {
                                final_checkpoint_sent = true;
                            }
                        }

                        // Continue waiting for close signal instead of breaking immediately
                        if !final_checkpoint_sent {
                            break;
                        }
                    }
                }

                // Check if we should close (this is now less relevant)
                if close_rx.try_recv().is_ok() {
                    log::info!("STT: Received close signal");
                    break;
                }
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
                "STT: Closing speech sender, sent {} chunks total",
                chunk_count
            );
            let _ = write.close().await; // Close the write half when done
        });

        // Get timeout from config before moving self
        let server_timeout = self.config.server_timeout;

        // This main task handles receiving messages and waits for final checkpoint
        let mut final_transcript = String::new();
        let mut message_count = 0;
        let mut received_final_checkpoint = false;
        let mut last_message_time = Instant::now();
        let mut segments: HashMap<u32, String> = HashMap::new();

        log::info!("STT: Starting to listen for server responses");

        // Process messages until we get final checkpoint or timeout
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
            if last_message_time.elapsed() > Duration::from_secs(10) {
                log::warn!("STT: No messages received for 10 seconds");
                return Err(STTError::Streaming(
                    "No response for 10 seconds".to_string(),
                ));
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

    /// Transcribe streaming audio with initial context chunks
    /// This allows us to include recent audio chunks that were captured during wakeword detection
    pub async fn transcribe_stream_with_context(
        self: Arc<Self>,
        mut speech_receiver: broadcast::Receiver<SpeechChunk>,
        context_chunks: Vec<SpeechChunk>,
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

        // Clone self for the async block
        let self_clone = Arc::clone(&self);

        // Spawn a dedicated task for sending audio
        let sender_handle = tokio::spawn(async move {
            let mut chunk_count = 0;
            let mut final_checkpoint_sent = false;

            // First, send context chunks
            log::info!("STT: Sending {} context chunks", context_chunks.len());
            for chunk in context_chunks {
                chunk_count += 1;
                let pcm_data = self_clone.samples_to_pcm(&chunk.samples_f32).unwrap();
                let samples_count = chunk.samples_f32.len();
                let duration_ms = (samples_count as f32 / 16000.0) * 1000.0;

                log::debug!(
                    "STT: Sending context chunk {} ({} samples = {:.1}ms = {} bytes)",
                    chunk_count,
                    samples_count,
                    duration_ms,
                    pcm_data.len()
                );

                if write.send(Message::Binary(pcm_data.into())).await.is_err() {
                    log::warn!("STT: Failed to send context chunk {}", chunk_count);
                    break;
                }

                // Send immediately without artificial pacing delays
            }

            // Then continue with live stream
            loop {
                // If we've sent the final checkpoint, just wait for close signal or channel close
                if final_checkpoint_sent {
                    match close_rx.try_recv() {
                        Ok(_) => {
                            log::info!("STT: Received close signal after final checkpoint");
                            break;
                        }
                        Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                            continue;
                        }
                        Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                            log::info!("STT: Close channel disconnected");
                            break;
                        }
                    }
                }

                match speech_receiver.recv().await {
                    Ok(speech_chunk) => {
                        match speech_chunk.speech_event {
                            SpeechEvent::StartedSpeaking | SpeechEvent::Speaking => {
                                chunk_count += 1;
                                let pcm_data = self_clone
                                    .samples_to_pcm(&speech_chunk.samples_f32)
                                    .unwrap();
                                let samples_count = speech_chunk.samples_f32.len();
                                let duration_ms = (samples_count as f32 / 16000.0) * 1000.0;

                                log::debug!(
                                    "STT: Sending live chunk {} ({} samples = {:.1}ms = {} bytes)",
                                    chunk_count,
                                    samples_count,
                                    duration_ms,
                                    pcm_data.len()
                                );

                                // Send immediately without artificial pacing delays

                                if write.send(Message::Binary(pcm_data.into())).await.is_err() {
                                    log::warn!("STT: Failed to send live chunk {}", chunk_count);
                                    break;
                                }
                            }
                            SpeechEvent::StoppedSpeaking => {
                                log::info!("STT: Received StoppedSpeaking event, waiting briefly for potential continuation...");

                                // Wait a brief moment to see if speech resumes
                                tokio::time::sleep(Duration::from_millis(200)).await;

                                // Check if we received any new speech chunks during the pause
                                let mut should_send_checkpoint = true;
                                while let Ok(new_chunk) = speech_receiver.try_recv() {
                                    match new_chunk.speech_event {
                                        SpeechEvent::StartedSpeaking | SpeechEvent::Speaking => {
                                            // Speech resumed, continue processing
                                            chunk_count += 1;
                                            let pcm_data = self_clone
                                                .samples_to_pcm(&new_chunk.samples_f32)
                                                .unwrap();
                                            let samples_count = new_chunk.samples_f32.len();
                                            let duration_ms =
                                                (samples_count as f32 / 16000.0) * 1000.0;

                                            log::debug!(
                                                "STT: Speech resumed! Sending chunk {} ({} samples = {:.1}ms = {} bytes)",
                                                chunk_count,
                                                samples_count,
                                                duration_ms,
                                                pcm_data.len()
                                            );

                                            if write
                                                .send(Message::Binary(pcm_data.into()))
                                                .await
                                                .is_err()
                                            {
                                                log::warn!(
                                                    "STT: Failed to send resumed speech chunk {}",
                                                    chunk_count
                                                );
                                                break;
                                            }
                                            should_send_checkpoint = false;
                                        }
                                        SpeechEvent::StoppedSpeaking => {
                                            continue;
                                        }
                                    }
                                }

                                if should_send_checkpoint {
                                    log::info!("STT: No speech resumption detected, sending final checkpoint");

                                    let final_checkpoint = json!({"checkpoint_id": "final"});
                                    let checkpoint_msg =
                                        serde_json::to_string(&final_checkpoint).unwrap();

                                    if let Err(e) =
                                        write.send(Message::Text(checkpoint_msg.into())).await
                                    {
                                        log::warn!("STT: Failed to send final checkpoint: {}", e);
                                        break;
                                    } else {
                                        log::info!("STT: Final checkpoint sent successfully, waiting for acknowledgment");
                                        final_checkpoint_sent = true;
                                    }
                                } else {
                                    log::info!(
                                        "STT: Speech resumed after pause, continuing transcription"
                                    );
                                }
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        log::warn!("STT: Speech receiver lagged, skipped {} messages", skipped);
                        continue;
                    }
                    Err(_) => {
                        if !final_checkpoint_sent {
                            log::info!(
                                "STT: Speech channel closed, sent {} chunks total",
                                chunk_count
                            );

                            let final_checkpoint = json!({"checkpoint_id": "final"});
                            let checkpoint_msg = serde_json::to_string(&final_checkpoint).unwrap();

                            if let Err(e) = write.send(Message::Text(checkpoint_msg.into())).await {
                                log::warn!(
                                    "STT: Failed to send final checkpoint on channel close: {}",
                                    e
                                );
                            } else {
                                final_checkpoint_sent = true;
                            }
                        }

                        if !final_checkpoint_sent {
                            break;
                        }
                    }
                }

                if close_rx.try_recv().is_ok() {
                    log::info!("STT: Received close signal");
                    break;
                }
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
                "STT: Closing speech sender, sent {} chunks total",
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
        let mut segments: HashMap<u32, String> = HashMap::new();

        log::info!("STT: Starting to listen for server responses");

        // Main message receiving loop
        loop {
            tokio::select! {
                message = read.next() => {
                    match message {
                        Some(Ok(Message::Text(text))) => {
                            message_count += 1;
                            let text_str = text.to_string();
                            log::debug!("STT: Received text message {}: {}", message_count, text_str);

                                                                                    // Check for checkpoint messages first, before trying to parse as StreamingResponse
                            if text_str.contains("checkpoint_id") {
                                log::debug!("STT: Checking checkpoint message: '{}'", text_str);

                                // Parse as JSON to properly check for final checkpoint
                                if let Ok(json_value) = serde_json::from_str::<serde_json::Value>(&text_str) {
                                    if json_value.get("checkpoint_id").and_then(|v| v.as_str()) == Some("final") {
                                        let final_ack_time = Instant::now();
                                        log::info!("STT: Received final checkpoint acknowledgment at {:?} - transcription complete", final_ack_time);
                                        received_final_checkpoint = true;
                                        let _ = final_tx.send(()).await;

                                        // Exit immediately - no need to wait for more messages
                                        log::info!("STT: Breaking out of message loop at {:?} with transcript: '{}'", final_ack_time, final_transcript);
                                        break;
                                    } else {
                                        log::debug!("STT: Received non-final checkpoint: {}", text_str);
                                    }
                                } else {
                                    log::debug!("STT: Failed to parse checkpoint message as JSON: {}", text_str);
                                }
                            } else if let Ok(response) = serde_json::from_str::<StreamingResponse>(&text_str) {
                                if let Some(text_content) = response.text {
                                    final_transcript = text_content.clone();
                                    log::info!("STT: Updated transcript: '{}'", text_content);
                                }
                                log::debug!("STT: Parsed as StreamingResponse");
                            } else {
                                log::debug!("STT: Received other message: {}", text_str);
                            }
                        }
                        Some(Ok(Message::Close(_))) => {
                            log::info!("STT: Server closed connection");
                            break;
                        }
                        Some(Ok(_)) => {
                            // Don't log non-text messages if we've already received final checkpoint
                            // This prevents spam from ping/pong frames after completion
                            if !received_final_checkpoint {
                                log::debug!("STT: Received non-text message");
                            }
                        }
                        Some(Err(e)) => {
                            log::error!("STT: WebSocket error: {}", e);
                            break;
                        }
                        None => {
                            log::info!("STT: WebSocket stream ended");
                            break;
                        }
                    }
                }
                _ = tokio::time::sleep(server_timeout) => {
                    log::warn!("STT: Server timeout after {:?}", server_timeout);
                    break;
                }
            }
        }

        log::info!("STT: Exited message loop, cleaning up...");

        // Signal the sender to close
        let _ = close_tx.send(()).await;

        // Wait for sender task to complete
        log::info!("STT: Waiting for sender task to complete...");
        let _ = sender_handle.await;
        log::info!("STT: Sender task completed");

        if !received_final_checkpoint {
            log::warn!("STT: Did not receive final checkpoint acknowledgment from server");
        }

        if final_transcript.trim().is_empty() {
            return Err(STTError::Streaming("No transcript received".to_string()));
        }

        let return_time = Instant::now();
        log::info!(
            "STT: Returning final transcript at {:?}: '{}'",
            return_time,
            final_transcript.trim()
        );
        Ok(final_transcript.trim().to_string())
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
        assert_eq!(config.language, Some("en".to_string()));
        assert_eq!(config.temperature, Some(0.0));
        assert_eq!(
            config.prompt,
            Some("The user will say 'Hey Mycroft' followed by a question or command.".to_string())
        );
        assert_eq!(config.server_timeout, Duration::from_millis(30000));
    }

    #[tokio::test]
    async fn test_stt_creation() {
        let stt = FireworksSTT::new("test_key".to_string());
        assert_eq!(stt.api_key, "test_key");
        assert_eq!(stt.config.server_timeout, Duration::from_millis(30000));
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
}
