use super::types::STTError;
use serde_json;
use std::net::TcpStream;
use std::time::Duration;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{connect, Message, WebSocket};
use url::Url;

pub struct WebSocketSender {
    ws: WebSocket<MaybeTlsStream<TcpStream>>,
}

impl WebSocketSender {
    pub fn new(api_key: String) -> Result<Self, STTError> {
        // Create WebSocket URL with query parameters
        let mut url = Url::parse(
            "wss://audio-streaming.us-virginia-1.direct.fireworks.ai/v1/audio/transcriptions/streaming",
        ).map_err(|e| STTError::WebSocketError(format!("Invalid URL: {}", e)))?;

        // Add query parameters for Fireworks API
        url.query_pairs_mut()
            .append_pair("response_format", "verbose_json")
            .append_pair("Authorization", &api_key)
            .append_pair("temperature", "0.0")
            .append_pair("prompt", "Transcribe the following audio accurately.");

        log::info!("🌐 Connecting to Fireworks STT WebSocket: {}", url.as_str());

        // Connect to WebSocket (blocking)
        let (ws, response) = connect(url.as_str())
            .map_err(|e| STTError::WebSocketError(format!("Connection failed: {}", e)))?;

        log::info!("✅ WebSocket connected, status: {}", response.status());

        Ok(Self { ws })
    }

    /// Send audio data to the WebSocket
    pub fn send_audio_data(&mut self, audio_data: Vec<u8>) -> Result<(), STTError> {
        if !audio_data.is_empty() {
            let data_len = audio_data.len();
            self.ws
                .send(Message::Binary(audio_data))
                .map_err(|e| STTError::WebSocketError(format!("Send failed: {}", e)))?;
            log::trace!("📤 Sent {} bytes of audio data", data_len);
        } else {
            // Empty audio data means end-of-stream - send the checkpoint marker
            let final_checkpoint = serde_json::json!({"checkpoint_id": "final"});
            log::info!("📡 Sending end-of-stream checkpoint: {}", final_checkpoint);
            self.ws
                .send(Message::Text(final_checkpoint.to_string()))
                .map_err(|e| {
                    STTError::WebSocketError(format!("Failed to send end checkpoint: {}", e))
                })?;
        }
        Ok(())
    }

    /// Clone this WebSocket for reading responses (creates a separate handle)
    pub fn clone_for_reading(&mut self) -> Result<WebSocketSender, STTError> {
        // This is a simplified approach - in practice we'd need to properly share the WebSocket
        // For now, we'll assume the WebSocket can be used from the same struct
        // We'll modify the read_response method to be called on the same instance
        Err(STTError::WebSocketError(
            "Clone not implemented - use read_response on same instance".into(),
        ))
    }

    /// Read a response from the WebSocket (with timeout via non-blocking)
    pub fn read_response(&mut self) -> Result<Option<String>, STTError> {
        // Use non-blocking read pattern instead of set_read_timeout

        match self.ws.read() {
            Ok(Message::Text(text)) => {
                log::debug!("📨 Received server message: {}", text);
                Ok(Some(text))
            }
            Ok(Message::Close(_)) => {
                log::info!("🔚 Server closed WebSocket connection");
                Ok(None) // Connection closed
            }
            Ok(Message::Ping(data)) => {
                log::trace!("🏓 Received ping, sending pong");
                self.ws
                    .send(Message::Pong(data))
                    .map_err(|e| STTError::WebSocketError(format!("Pong failed: {}", e)))?;
                // Continue reading after pong
                self.read_response()
            }
            Ok(_) => {
                // Other message types, continue reading
                self.read_response()
            }
            Err(tungstenite::Error::Io(ref e)) if e.kind() == std::io::ErrorKind::WouldBlock => {
                // Timeout - no message available
                Ok(Some(String::new())) // Return empty string to indicate no message yet
            }
            Err(e) => {
                log::warn!("❌ WebSocket read error: {}", e);
                Err(STTError::WebSocketError(format!("Read failed: {}", e)))
            }
        }
    }

    /// Close the WebSocket connection cleanly
    pub fn close(&mut self) -> Result<(), STTError> {
        self.ws
            .close(None)
            .map_err(|e| STTError::WebSocketError(format!("Close failed: {}", e)))?;
        log::info!("🔚 WebSocket closed cleanly");
        Ok(())
    }
}
