use serde::{Deserialize, Serialize};
use std::io::{BufReader, BufWriter, Read, Write};
use std::net::TcpStream;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ProtocolError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Invalid message type: {0}")]
    InvalidMessageType(u8),

    #[error("Invalid payload size: {0}")]
    InvalidPayloadSize(u32),

    #[error("Invalid string encoding")]
    InvalidString,

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Message types for the wakeword protocol
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MessageType {
    // Client → Server
    SubscribeWakeword = 0x01,
    UnsubscribeWakeword = 0x02,

    // Server → Client
    WakewordEvent = 0x10,
    SubscribeResponse = 0x11,
    UnsubscribeResponse = 0x12,
    ErrorResponse = 0x13,
}

impl TryFrom<u8> for MessageType {
    type Error = ProtocolError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x01 => Ok(MessageType::SubscribeWakeword),
            0x02 => Ok(MessageType::UnsubscribeWakeword),
            0x10 => Ok(MessageType::WakewordEvent),
            0x11 => Ok(MessageType::SubscribeResponse),
            0x12 => Ok(MessageType::UnsubscribeResponse),
            0x13 => Ok(MessageType::ErrorResponse),
            _ => Err(ProtocolError::InvalidMessageType(value)),
        }
    }
}

/// Wakeword detection event
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WakewordEvent {
    /// Name of the wake word model that detected the event
    pub model_name: String,
    /// Confidence score from 0.0 to 1.0
    pub confidence: f32,
    /// Unix timestamp in milliseconds when the event occurred
    pub timestamp: u64,
    /// ID of the client that detected the wake word
    pub client_id: String,
}

impl WakewordEvent {
    pub fn new(model_name: String, confidence: f32, client_id: String) -> Self {
        Self {
            model_name,
            confidence,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            client_id,
        }
    }
}

/// Protocol messages
#[derive(Debug, Clone)]
pub enum Message {
    SubscribeWakeword,
    UnsubscribeWakeword,
    WakewordEvent(WakewordEvent),
    SubscribeResponse { success: bool, message: String },
    UnsubscribeResponse { success: bool, message: String },
    ErrorResponse { error: String },
}

impl Message {
    /// Get the message type for this message
    pub fn message_type(&self) -> MessageType {
        match self {
            Message::SubscribeWakeword => MessageType::SubscribeWakeword,
            Message::UnsubscribeWakeword => MessageType::UnsubscribeWakeword,
            Message::WakewordEvent(_) => MessageType::WakewordEvent,
            Message::SubscribeResponse { .. } => MessageType::SubscribeResponse,
            Message::UnsubscribeResponse { .. } => MessageType::UnsubscribeResponse,
            Message::ErrorResponse { .. } => MessageType::ErrorResponse,
        }
    }

    /// Serialize message to bytes (JSON payload for data messages)
    pub fn to_bytes(&self) -> Result<Vec<u8>, ProtocolError> {
        let payload = match self {
            Message::SubscribeWakeword | Message::UnsubscribeWakeword => {
                Vec::new() // No payload for simple commands
            }
            Message::WakewordEvent(event) => serde_json::to_vec(event)?,
            Message::SubscribeResponse { success, message } => {
                serde_json::to_vec(&serde_json::json!({
                    "success": success,
                    "message": message
                }))?
            }
            Message::UnsubscribeResponse { success, message } => {
                serde_json::to_vec(&serde_json::json!({
                    "success": success,
                    "message": message
                }))?
            }
            Message::ErrorResponse { error } => serde_json::to_vec(&serde_json::json!({
                "error": error
            }))?,
        };

        // Build binary message: [message_type: u8][payload_size: u32][payload: bytes]
        let mut bytes = Vec::new();
        bytes.push(self.message_type() as u8);
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&payload);

        Ok(bytes)
    }

    /// Deserialize message from message type and payload
    pub fn from_bytes(message_type: MessageType, payload: &[u8]) -> Result<Self, ProtocolError> {
        match message_type {
            MessageType::SubscribeWakeword => Ok(Message::SubscribeWakeword),
            MessageType::UnsubscribeWakeword => Ok(Message::UnsubscribeWakeword),
            MessageType::WakewordEvent => {
                let event: WakewordEvent = serde_json::from_slice(payload)?;
                Ok(Message::WakewordEvent(event))
            }
            MessageType::SubscribeResponse => {
                let data: serde_json::Value = serde_json::from_slice(payload)?;
                Ok(Message::SubscribeResponse {
                    success: data["success"].as_bool().unwrap_or(false),
                    message: data["message"].as_str().unwrap_or("").to_string(),
                })
            }
            MessageType::UnsubscribeResponse => {
                let data: serde_json::Value = serde_json::from_slice(payload)?;
                Ok(Message::UnsubscribeResponse {
                    success: data["success"].as_bool().unwrap_or(false),
                    message: data["message"].as_str().unwrap_or("").to_string(),
                })
            }
            MessageType::ErrorResponse => {
                let data: serde_json::Value = serde_json::from_slice(payload)?;
                Ok(Message::ErrorResponse {
                    error: data["error"]
                        .as_str()
                        .unwrap_or("Unknown error")
                        .to_string(),
                })
            }
        }
    }
}

/// TCP connection wrapper for the wakeword protocol
pub struct Connection {
    reader: BufReader<TcpStream>,
    writer: BufWriter<TcpStream>,
}

impl Connection {
    /// Create a new connection from a TCP stream
    pub fn new(stream: TcpStream) -> Result<Self, ProtocolError> {
        let read_stream = stream.try_clone()?;
        let write_stream = stream;

        Ok(Connection {
            reader: BufReader::new(read_stream),
            writer: BufWriter::new(write_stream),
        })
    }

    /// Read a message from the connection
    pub fn read_message(&mut self) -> Result<Message, ProtocolError> {
        // Read message type (1 byte)
        let mut type_byte = [0u8; 1];
        self.reader.read_exact(&mut type_byte)?;
        let message_type = MessageType::try_from(type_byte[0])?;

        // Read payload size (4 bytes, little endian)
        let mut size_bytes = [0u8; 4];
        self.reader.read_exact(&mut size_bytes)?;
        let payload_size = u32::from_le_bytes(size_bytes);

        // Validate payload size (prevent DoS attacks)
        if payload_size > 1024 * 1024 {
            // 1MB max
            return Err(ProtocolError::InvalidPayloadSize(payload_size));
        }

        // Read payload
        let mut payload = vec![0u8; payload_size as usize];
        if payload_size > 0 {
            self.reader.read_exact(&mut payload)?;
        }

        // Deserialize message
        Message::from_bytes(message_type, &payload)
    }

    /// Write a message to the connection
    pub fn write_message(&mut self, message: &Message) -> Result<(), ProtocolError> {
        let bytes = message.to_bytes()?;
        self.writer.write_all(&bytes)?;
        self.writer.flush()?;
        Ok(())
    }
}
