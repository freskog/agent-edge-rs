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
}

/// Message types for our binary protocol
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MessageType {
    // Client → Server
    SubscribeAudio = 0x01,
    UnsubscribeAudio = 0x02,
    PlayAudio = 0x03,
    EndStream = 0x04,
    AbortPlayback = 0x05,

    // Server → Client
    AudioChunk = 0x10,
    UnsubscribeResponse = 0x11,
    PlayResponse = 0x12,
    EndStreamResponse = 0x13,
    AbortResponse = 0x14,
    ErrorResponse = 0x15,
}

impl TryFrom<u8> for MessageType {
    type Error = ProtocolError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x01 => Ok(MessageType::SubscribeAudio),
            0x02 => Ok(MessageType::UnsubscribeAudio),
            0x03 => Ok(MessageType::PlayAudio),
            0x04 => Ok(MessageType::EndStream),
            0x05 => Ok(MessageType::AbortPlayback),
            0x10 => Ok(MessageType::AudioChunk),
            0x11 => Ok(MessageType::UnsubscribeResponse),
            0x12 => Ok(MessageType::PlayResponse),
            0x13 => Ok(MessageType::EndStreamResponse),
            0x14 => Ok(MessageType::AbortResponse),
            0x15 => Ok(MessageType::ErrorResponse),
            _ => Err(ProtocolError::InvalidMessageType(value)),
        }
    }
}

/// Messages that can be sent/received
#[derive(Debug, Clone)]
pub enum Message {
    // Client → Server
    SubscribeAudio,
    UnsubscribeAudio,
    PlayAudio {
        stream_id: String,
        audio_data: Vec<u8>,
    },
    EndStream {
        stream_id: String,
    },
    AbortPlayback {
        stream_id: String,
    },

    // Server → Client
    AudioChunk {
        audio_data: Vec<u8>,
        timestamp_ms: u64,
    },
    UnsubscribeResponse {
        success: bool,
        message: String,
    },
    PlayResponse {
        success: bool,
        message: String,
    },
    EndStreamResponse {
        success: bool,
        message: String,
        chunks_played: u32,
    },
    AbortResponse {
        success: bool,
        message: String,
    },
    ErrorResponse {
        message: String,
    },
}

impl Message {
    pub fn message_type(&self) -> MessageType {
        match self {
            Message::SubscribeAudio => MessageType::SubscribeAudio,
            Message::UnsubscribeAudio => MessageType::UnsubscribeAudio,
            Message::PlayAudio { .. } => MessageType::PlayAudio,
            Message::EndStream { .. } => MessageType::EndStream,
            Message::AbortPlayback { .. } => MessageType::AbortPlayback,
            Message::AudioChunk { .. } => MessageType::AudioChunk,
            Message::UnsubscribeResponse { .. } => MessageType::UnsubscribeResponse,
            Message::PlayResponse { .. } => MessageType::PlayResponse,
            Message::EndStreamResponse { .. } => MessageType::EndStreamResponse,
            Message::AbortResponse { .. } => MessageType::AbortResponse,
            Message::ErrorResponse { .. } => MessageType::ErrorResponse,
        }
    }

    /// Serialize message to bytes
    pub fn to_bytes(&self) -> Result<Vec<u8>, ProtocolError> {
        let mut payload = Vec::new();

        match self {
            Message::SubscribeAudio => {
                // No payload for subscribe
            }
            Message::UnsubscribeAudio => {
                // No payload for unsubscribe
            }
            Message::PlayAudio {
                stream_id,
                audio_data,
            } => {
                write_string(&mut payload, stream_id)?;
                write_bytes(&mut payload, audio_data)?;
            }
            Message::EndStream { stream_id } => {
                write_string(&mut payload, stream_id)?;
            }
            Message::AbortPlayback { stream_id } => {
                write_string(&mut payload, stream_id)?;
            }
            Message::AudioChunk {
                audio_data,
                timestamp_ms,
            } => {
                payload.extend_from_slice(&timestamp_ms.to_le_bytes());
                write_bytes(&mut payload, audio_data)?;
            }
            Message::UnsubscribeResponse { success, message } => {
                payload.push(if *success { 1 } else { 0 });
                write_string(&mut payload, message)?;
            }
            Message::PlayResponse { success, message } => {
                payload.push(if *success { 1 } else { 0 });
                write_string(&mut payload, message)?;
            }
            Message::EndStreamResponse {
                success,
                message,
                chunks_played,
            } => {
                payload.push(if *success { 1 } else { 0 });
                payload.extend_from_slice(&chunks_played.to_le_bytes());
                write_string(&mut payload, message)?;
            }
            Message::AbortResponse { success, message } => {
                payload.push(if *success { 1 } else { 0 });
                write_string(&mut payload, message)?;
            }
            Message::ErrorResponse { message } => {
                write_string(&mut payload, message)?;
            }
        }

        // Build final message: [type:u8][length:u32][payload...]
        let mut message = Vec::new();
        message.push(self.message_type() as u8);
        message.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        message.extend_from_slice(&payload);

        Ok(message)
    }

    /// Deserialize message from bytes
    pub fn from_bytes(message_type: MessageType, payload: &[u8]) -> Result<Self, ProtocolError> {
        let mut reader = payload;

        match message_type {
            MessageType::SubscribeAudio => Ok(Message::SubscribeAudio),
            MessageType::UnsubscribeAudio => Ok(Message::UnsubscribeAudio),
            MessageType::PlayAudio => {
                let stream_id = read_string(&mut reader)?;
                let audio_data = read_bytes(&mut reader)?;
                Ok(Message::PlayAudio {
                    stream_id,
                    audio_data,
                })
            }
            MessageType::EndStream => {
                let stream_id = read_string(&mut reader)?;
                Ok(Message::EndStream { stream_id })
            }
            MessageType::AbortPlayback => {
                let stream_id = read_string(&mut reader)?;
                Ok(Message::AbortPlayback { stream_id })
            }
            MessageType::AudioChunk => {
                if reader.len() < 8 {
                    return Err(ProtocolError::InvalidPayloadSize(reader.len() as u32));
                }
                let timestamp_ms = u64::from_le_bytes([
                    reader[0], reader[1], reader[2], reader[3], reader[4], reader[5], reader[6],
                    reader[7],
                ]);
                reader = &reader[8..];
                let audio_data = read_bytes(&mut reader)?;
                Ok(Message::AudioChunk {
                    audio_data,
                    timestamp_ms,
                })
            }
            MessageType::UnsubscribeResponse => {
                if reader.is_empty() {
                    return Err(ProtocolError::InvalidPayloadSize(0));
                }
                let success = reader[0] != 0;
                reader = &reader[1..];
                let message = read_string(&mut reader)?;
                Ok(Message::UnsubscribeResponse { success, message })
            }
            MessageType::PlayResponse => {
                if reader.is_empty() {
                    return Err(ProtocolError::InvalidPayloadSize(0));
                }
                let success = reader[0] != 0;
                reader = &reader[1..];
                let message = read_string(&mut reader)?;
                Ok(Message::PlayResponse { success, message })
            }
            MessageType::EndStreamResponse => {
                if reader.len() < 5 {
                    return Err(ProtocolError::InvalidPayloadSize(reader.len() as u32));
                }
                let success = reader[0] != 0;
                let chunks_played =
                    u32::from_le_bytes([reader[1], reader[2], reader[3], reader[4]]);
                reader = &reader[5..];
                let message = read_string(&mut reader)?;
                Ok(Message::EndStreamResponse {
                    success,
                    message,
                    chunks_played,
                })
            }
            MessageType::AbortResponse => {
                if reader.is_empty() {
                    return Err(ProtocolError::InvalidPayloadSize(0));
                }
                let success = reader[0] != 0;
                reader = &reader[1..];
                let message = read_string(&mut reader)?;
                Ok(Message::AbortResponse { success, message })
            }
            MessageType::ErrorResponse => {
                let message = read_string(&mut reader)?;
                Ok(Message::ErrorResponse { message })
            }
        }
    }
}

/// Connection wrapper for reading/writing messages
pub struct Connection {
    reader: BufReader<TcpStream>,
    writer: BufWriter<TcpStream>,
}

impl Connection {
    pub fn new(stream: TcpStream) -> Result<Self, ProtocolError> {
        let reader_stream = stream.try_clone()?;
        let reader = BufReader::new(reader_stream);
        let writer = BufWriter::new(stream);

        Ok(Connection { reader, writer })
    }

    /// Read a message from the connection
    pub fn read_message(&mut self) -> Result<Message, ProtocolError> {
        // Read message type and length
        let mut header = [0u8; 5];
        self.reader.read_exact(&mut header)?;

        let message_type = MessageType::try_from(header[0])?;
        let payload_length = u32::from_le_bytes([header[1], header[2], header[3], header[4]]);

        // Sanity check payload length (max 16MB)
        if payload_length > 16 * 1024 * 1024 {
            return Err(ProtocolError::InvalidPayloadSize(payload_length));
        }

        // Read payload
        let mut payload = vec![0u8; payload_length as usize];
        if payload_length > 0 {
            self.reader.read_exact(&mut payload)?;
        }

        // Parse message
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

// Helper functions for reading/writing strings and byte arrays

fn write_string(buffer: &mut Vec<u8>, s: &str) -> Result<(), ProtocolError> {
    let bytes = s.as_bytes();
    let len = bytes.len() as u32;
    buffer.extend_from_slice(&len.to_le_bytes());
    buffer.extend_from_slice(bytes);
    Ok(())
}

fn read_string(reader: &mut &[u8]) -> Result<String, ProtocolError> {
    if reader.len() < 4 {
        return Err(ProtocolError::InvalidPayloadSize(reader.len() as u32));
    }

    let len = u32::from_le_bytes([reader[0], reader[1], reader[2], reader[3]]) as usize;
    *reader = &reader[4..];

    if reader.len() < len {
        return Err(ProtocolError::InvalidPayloadSize(reader.len() as u32));
    }

    let string_bytes = &reader[..len];
    *reader = &reader[len..];

    String::from_utf8(string_bytes.to_vec()).map_err(|_| ProtocolError::InvalidString)
}

fn write_bytes(buffer: &mut Vec<u8>, bytes: &[u8]) -> Result<(), ProtocolError> {
    let len = bytes.len() as u32;
    buffer.extend_from_slice(&len.to_le_bytes());
    buffer.extend_from_slice(bytes);
    Ok(())
}

fn read_bytes(reader: &mut &[u8]) -> Result<Vec<u8>, ProtocolError> {
    if reader.len() < 4 {
        return Err(ProtocolError::InvalidPayloadSize(reader.len() as u32));
    }

    let len = u32::from_le_bytes([reader[0], reader[1], reader[2], reader[3]]) as usize;
    *reader = &reader[4..];

    if reader.len() < len {
        return Err(ProtocolError::InvalidPayloadSize(reader.len() as u32));
    }

    let bytes = reader[..len].to_vec();
    *reader = &reader[len..];

    Ok(bytes)
}
