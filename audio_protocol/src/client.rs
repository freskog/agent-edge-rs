use crate::protocol::{Connection, Message, ProtocolError};
use log::{debug, error, info};
use std::net::TcpStream;
use std::time::Duration;

/// High-level TCP client for the audio API
pub struct AudioClient {
    connection: Connection,
    server_address: String,
}

impl AudioClient {
    /// Connect to the audio server
    pub fn connect(address: &str) -> Result<Self, ProtocolError> {
        info!("📡 Connecting to audio server at {}", address);

        let stream = TcpStream::connect(address)?;
        stream.set_read_timeout(Some(Duration::from_secs(30)))?;
        stream.set_write_timeout(Some(Duration::from_secs(10)))?;

        let connection = Connection::new(stream)?;

        info!("✅ Connected to audio server");

        Ok(AudioClient {
            connection,
            server_address: address.to_string(),
        })
    }

    /// Subscribe to audio capture stream
    pub fn subscribe_audio(&mut self) -> Result<(), ProtocolError> {
        debug!("📤 Sending SubscribeAudio message");

        let message = Message::SubscribeAudio;
        self.connection.write_message(&message)?;

        info!("🎤 Subscribed to audio capture");
        Ok(())
    }

    /// Unsubscribe from audio capture
    pub fn unsubscribe_audio(&mut self) -> Result<UnsubscribeResult, ProtocolError> {
        debug!("📤 Sending UnsubscribeAudio message");

        let message = Message::UnsubscribeAudio;
        self.connection.write_message(&message)?;

        // Drain any in-flight AudioChunk messages before expecting UnsubscribeResponse
        // This fixes the race condition with network latency
        loop {
            let response = self.connection.read_message()?;

            match response {
                Message::AudioChunk { .. } => {
                    // Drain in-flight audio chunks that were sent before unsubscribe took effect
                    debug!("🔄 Draining in-flight audio chunk during unsubscribe");
                    continue;
                }
                Message::UnsubscribeResponse { success, message } => {
                    if success {
                        info!("✅ Unsubscribed from audio capture: {}", message);
                    } else {
                        error!("❌ Unsubscribe failed: {}", message);
                    }

                    return Ok(UnsubscribeResult { success, message });
                }
                Message::ErrorResponse { message } => {
                    error!("❌ Server error: {}", message);
                    return Ok(UnsubscribeResult {
                        success: false,
                        message,
                    });
                }
                other => {
                    error!(
                        "❌ Unexpected response type during unsubscribe: {:?}",
                        other.message_type()
                    );
                    return Err(ProtocolError::InvalidMessageType(other.message_type() as u8));
                }
            }
        }
    }

    /// Read the next audio chunk (blocking)
    /// Returns None if an error response is received
    pub fn read_audio_chunk(&mut self) -> Result<Option<AudioChunk>, ProtocolError> {
        let message = self.connection.read_message()?;

        match message {
            Message::AudioChunk {
                audio_data,
                timestamp_ms,
            } => {
                debug!(
                    "📥 Received audio chunk: {} bytes (timestamp: {})",
                    audio_data.len(),
                    timestamp_ms
                );

                Ok(Some(AudioChunk {
                    data: audio_data,
                    timestamp_ms,
                }))
            }
            Message::ErrorResponse { message } => {
                error!("❌ Server error: {}", message);
                Ok(None)
            }
            other => {
                error!("❌ Unexpected message type: {:?}", other.message_type());
                Err(ProtocolError::InvalidMessageType(other.message_type() as u8))
            }
        }
    }

    /// Play audio chunk and wait for response
    pub fn play_audio_chunk(
        &mut self,
        stream_id: &str,
        audio_data: Vec<u8>,
    ) -> Result<PlayResult, ProtocolError> {
        debug!(
            "📤 Sending audio chunk: {} bytes to stream '{}'",
            audio_data.len(),
            stream_id
        );

        let message = Message::PlayAudio {
            stream_id: stream_id.to_string(),
            audio_data,
        };

        self.connection.write_message(&message)?;

        // Wait for response, but handle any unexpected delayed messages
        loop {
            let response = self.connection.read_message()?;

            match response {
                Message::PlayResponse { success, message } => {
                    if success {
                        debug!("✅ Play response: {}", message);
                    } else {
                        error!("❌ Play failed: {}", message);
                    }

                    return Ok(PlayResult { success, message });
                }
                Message::ErrorResponse { message } => {
                    error!("❌ Server error: {}", message);
                    return Ok(PlayResult {
                        success: false,
                        message,
                    });
                }
                Message::UnsubscribeResponse { .. } => {
                    // Drain delayed UnsubscribeResponse messages that arrived after we started playback
                    debug!("🔄 Draining delayed UnsubscribeResponse during playback");
                    continue;
                }
                other => {
                    error!(
                        "❌ Unexpected response type during playback: {:?}",
                        other.message_type()
                    );
                    return Err(ProtocolError::InvalidMessageType(other.message_type() as u8));
                }
            }
        }
    }

    /// End audio stream and wait for completion
    pub fn end_stream(&mut self, stream_id: &str) -> Result<EndStreamResult, ProtocolError> {
        info!("⏹️  Ending stream: {}", stream_id);

        let message = Message::EndStream {
            stream_id: stream_id.to_string(),
        };

        self.connection.write_message(&message)?;

        // Wait for response, but handle any unexpected delayed messages
        loop {
            let response = self.connection.read_message()?;

            match response {
                Message::EndStreamResponse {
                    success,
                    message,
                    chunks_played,
                } => {
                    if success {
                        info!(
                            "✅ Stream ended: {} (played {} chunks)",
                            message, chunks_played
                        );
                    } else {
                        error!("❌ End stream failed: {}", message);
                    }

                    return Ok(EndStreamResult {
                        success,
                        message,
                        chunks_played,
                    });
                }
                Message::ErrorResponse { message } => {
                    error!("❌ Server error: {}", message);
                    return Ok(EndStreamResult {
                        success: false,
                        message,
                        chunks_played: 0,
                    });
                }
                Message::PlayResponse { .. } => {
                    // Drain delayed PlayResponse messages
                    debug!("🔄 Draining delayed PlayResponse during stream end");
                    continue;
                }
                other => {
                    error!(
                        "❌ Unexpected response type during stream end: {:?}",
                        other.message_type()
                    );
                    return Err(ProtocolError::InvalidMessageType(other.message_type() as u8));
                }
            }
        }
    }

    /// Abort playback
    pub fn abort_playback(&mut self, stream_id: &str) -> Result<AbortResult, ProtocolError> {
        info!("🛑 Aborting playback: {}", stream_id);

        let message = Message::AbortPlayback {
            stream_id: stream_id.to_string(),
        };

        self.connection.write_message(&message)?;

        // Wait for response, but handle any unexpected delayed messages
        loop {
            let response = self.connection.read_message()?;

            match response {
                Message::AbortResponse { success, message } => {
                    if success {
                        info!("✅ Playback aborted: {}", message);
                    } else {
                        error!("❌ Abort failed: {}", message);
                    }

                    return Ok(AbortResult { success, message });
                }
                Message::ErrorResponse { message } => {
                    error!("❌ Server error: {}", message);
                    return Ok(AbortResult {
                        success: false,
                        message,
                    });
                }
                Message::PlayResponse { .. } => {
                    // Drain delayed PlayResponse messages
                    debug!("🔄 Draining delayed PlayResponse during abort");
                    continue;
                }
                other => {
                    error!(
                        "❌ Unexpected response type during abort: {:?}",
                        other.message_type()
                    );
                    return Err(ProtocolError::InvalidMessageType(other.message_type() as u8));
                }
            }
        }
    }

    /// Get the server address
    pub fn server_address(&self) -> &str {
        &self.server_address
    }
}

/// Audio chunk received from the server
#[derive(Debug, Clone)]
pub struct AudioChunk {
    pub data: Vec<u8>,
    pub timestamp_ms: u64,
}

impl AudioChunk {
    /// Get the size of the audio data in bytes
    pub fn size_bytes(&self) -> usize {
        self.data.len()
    }

    /// Get the number of samples (assuming s16le format)
    pub fn sample_count(&self) -> usize {
        self.data.len() / 2
    }
}

/// Result of a play audio operation
#[derive(Debug, Clone)]
pub struct PlayResult {
    pub success: bool,
    pub message: String,
}

/// Result of an end stream operation
#[derive(Debug, Clone)]
pub struct EndStreamResult {
    pub success: bool,
    pub message: String,
    pub chunks_played: u32,
}

/// Result of an unsubscribe operation
#[derive(Debug, Clone)]
pub struct UnsubscribeResult {
    pub success: bool,
    pub message: String,
}

/// Result of an abort operation
#[derive(Debug, Clone)]
pub struct AbortResult {
    pub success: bool,
    pub message: String,
}
