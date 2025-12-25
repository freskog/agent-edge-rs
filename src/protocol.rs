use std::io::{Read, Write};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ProtocolError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Invalid payload size: {0}")]
    InvalidPayloadSize(u32),

    #[error("UTF-8 encoding error: {0}")]
    Utf8(#[from] std::str::Utf8Error),

    #[error("Invalid message type: {0}")]
    InvalidMessageType(u8),
}

/// Consumer message types (Port 8080)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ConsumerMessageType {
    // Audio Crate → Client
    Error = 0x11,
    Audio = 0x12,
    WakewordDetected = 0x15,
}

impl TryFrom<u8> for ConsumerMessageType {
    type Error = ProtocolError;

    fn try_from(value: u8) -> Result<Self, ProtocolError> {
        match value {
            0x11 => Ok(ConsumerMessageType::Error),
            0x12 => Ok(ConsumerMessageType::Audio),
            0x15 => Ok(ConsumerMessageType::WakewordDetected),
            _ => Err(ProtocolError::InvalidMessageType(value)),
        }
    }
}

/// Producer message types (Port 8081)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ProducerMessageType {
    // Client → Audio Crate
    Play = 0x20,
    Stop = 0x21,
    EndOfStream = 0x22,

    // Audio Crate → Client
    Error = 0x31,
    PlaybackComplete = 0x32,
}

impl TryFrom<u8> for ProducerMessageType {
    type Error = ProtocolError;

    fn try_from(value: u8) -> Result<Self, ProtocolError> {
        match value {
            0x20 => Ok(ProducerMessageType::Play),
            0x21 => Ok(ProducerMessageType::Stop),
            0x22 => Ok(ProducerMessageType::EndOfStream),
            0x31 => Ok(ProducerMessageType::Error),
            0x32 => Ok(ProducerMessageType::PlaybackComplete),
            _ => Err(ProtocolError::InvalidMessageType(value)),
        }
    }
}

/// Consumer protocol messages
#[derive(Debug, Clone)]
pub enum ConsumerMessage {
    // Audio Crate → Client
    Error {
        message: String,
    },
    Audio {
        data: Vec<u8>,
        speech_detected: bool, // VAD result for this chunk
        timestamp: u64,        // When this chunk was captured (ms since epoch)
    },
    WakewordDetected {
        model: String,
        timestamp: u64,
        spotify_was_paused: bool, // NEW: Whether Spotify was paused for this wakeword
    },
}

/// Producer protocol messages
#[derive(Debug, Clone)]
pub enum ProducerMessage {
    // Client → Audio Crate
    Play { data: Vec<u8> },
    Stop { timestamp: u64 },
    EndOfStream { timestamp: u64 },

    // Audio Crate → Client
    Error { message: String },
    PlaybackComplete { timestamp: u64 },
}

impl ConsumerMessage {
    /// Get current timestamp in milliseconds since epoch
    pub fn current_timestamp() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    /// Serialize message to binary format: [MessageType: u8][PayloadLength: u32][Payload: bytes]
    pub fn to_bytes(&self) -> Result<Vec<u8>, ProtocolError> {
        let mut bytes = Vec::new();

        match self {
            ConsumerMessage::Error { message } => {
                bytes.push(ConsumerMessageType::Error as u8);
                let msg_bytes = message.as_bytes();
                bytes.extend_from_slice(&(msg_bytes.len() as u32).to_le_bytes());
                bytes.extend_from_slice(msg_bytes);
            }
            ConsumerMessage::Audio {
                data,
                speech_detected,
                timestamp,
            } => {
                bytes.push(ConsumerMessageType::Audio as u8);
                // Payload: [timestamp: u64][speech_detected: u8][data_length: u32][data: bytes]
                let payload_len = 8 + 1 + 4 + data.len();
                bytes.extend_from_slice(&(payload_len as u32).to_le_bytes());
                bytes.extend_from_slice(&timestamp.to_le_bytes());
                bytes.push(if *speech_detected { 1u8 } else { 0u8 });
                bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
                bytes.extend_from_slice(data);
            }
            ConsumerMessage::WakewordDetected {
                model,
                timestamp,
                spotify_was_paused,
            } => {
                bytes.push(ConsumerMessageType::WakewordDetected as u8);
                // Payload: [timestamp: u64][spotify_was_paused: u8][model_len: u32][model: bytes]
                let model_bytes = model.as_bytes();
                let payload_len = 8 + 1 + 4 + model_bytes.len(); // u64 + u8 + u32 + string
                bytes.extend_from_slice(&(payload_len as u32).to_le_bytes());
                bytes.extend_from_slice(&timestamp.to_le_bytes());
                bytes.push(if *spotify_was_paused { 1u8 } else { 0u8 });
                bytes.extend_from_slice(&(model_bytes.len() as u32).to_le_bytes());
                bytes.extend_from_slice(model_bytes);
            }
        }

        Ok(bytes)
    }

    /// Deserialize message from binary format
    pub fn from_bytes(
        msg_type: ConsumerMessageType,
        payload: &[u8],
    ) -> Result<Self, ProtocolError> {
        match msg_type {
            ConsumerMessageType::Error => {
                let message = String::from_utf8(payload.to_vec())
                    .map_err(|_| ProtocolError::Utf8(std::str::from_utf8(payload).unwrap_err()))?;
                Ok(ConsumerMessage::Error { message })
            }
            ConsumerMessageType::Audio => {
                if payload.len() < 13 {
                    // minimum: u64 + u8 + u32
                    return Err(ProtocolError::InvalidPayloadSize(payload.len() as u32));
                }

                let timestamp = u64::from_le_bytes([
                    payload[0], payload[1], payload[2], payload[3], payload[4], payload[5],
                    payload[6], payload[7],
                ]);
                let speech_detected = payload[8] != 0;
                let data_length =
                    u32::from_le_bytes([payload[9], payload[10], payload[11], payload[12]])
                        as usize;

                if payload.len() < 13 + data_length {
                    return Err(ProtocolError::InvalidPayloadSize(payload.len() as u32));
                }

                let data = payload[13..13 + data_length].to_vec();
                Ok(ConsumerMessage::Audio {
                    data,
                    speech_detected,
                    timestamp,
                })
            }
            ConsumerMessageType::WakewordDetected => {
                if payload.len() < 13 {
                    // minimum: u64 + u8 + u32
                    return Err(ProtocolError::InvalidPayloadSize(payload.len() as u32));
                }

                let timestamp = u64::from_le_bytes([
                    payload[0], payload[1], payload[2], payload[3], payload[4], payload[5],
                    payload[6], payload[7],
                ]);

                let spotify_was_paused = payload[8] != 0;

                let model_len =
                    u32::from_le_bytes([payload[9], payload[10], payload[11], payload[12]])
                        as usize;

                if payload.len() < 13 + model_len {
                    return Err(ProtocolError::InvalidPayloadSize(payload.len() as u32));
                }

                let model =
                    String::from_utf8(payload[13..13 + model_len].to_vec()).map_err(|_| {
                        ProtocolError::Utf8(
                            std::str::from_utf8(&payload[13..13 + model_len]).unwrap_err(),
                        )
                    })?;

                Ok(ConsumerMessage::WakewordDetected {
                    model,
                    timestamp,
                    spotify_was_paused,
                })
            }
        }
    }
}

impl ProducerMessage {
    /// Get current timestamp in milliseconds since epoch
    pub fn current_timestamp() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    /// Serialize message to binary format: [MessageType: u8][PayloadLength: u32][Payload: bytes]
    pub fn to_bytes(&self) -> Result<Vec<u8>, ProtocolError> {
        let mut bytes = Vec::new();

        match self {
            ProducerMessage::Play { data } => {
                bytes.push(ProducerMessageType::Play as u8);
                bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
                bytes.extend_from_slice(data);
            }
            ProducerMessage::Stop { timestamp } => {
                bytes.push(ProducerMessageType::Stop as u8);
                bytes.extend_from_slice(&8u32.to_le_bytes()); // payload size: u64
                bytes.extend_from_slice(&timestamp.to_le_bytes());
            }
            ProducerMessage::EndOfStream { timestamp } => {
                bytes.push(ProducerMessageType::EndOfStream as u8);
                bytes.extend_from_slice(&8u32.to_le_bytes()); // payload size: u64
                bytes.extend_from_slice(&timestamp.to_le_bytes());
            }
            ProducerMessage::Error { message } => {
                bytes.push(ProducerMessageType::Error as u8);
                let msg_bytes = message.as_bytes();
                bytes.extend_from_slice(&(msg_bytes.len() as u32).to_le_bytes());
                bytes.extend_from_slice(msg_bytes);
            }
            ProducerMessage::PlaybackComplete { timestamp } => {
                bytes.push(ProducerMessageType::PlaybackComplete as u8);
                bytes.extend_from_slice(&8u32.to_le_bytes()); // payload size: u64
                bytes.extend_from_slice(&timestamp.to_le_bytes());
            }
        }

        Ok(bytes)
    }

    /// Deserialize message from binary format
    pub fn from_bytes(
        msg_type: ProducerMessageType,
        payload: &[u8],
    ) -> Result<Self, ProtocolError> {
        match msg_type {
            ProducerMessageType::Play => Ok(ProducerMessage::Play {
                data: payload.to_vec(),
            }),
            ProducerMessageType::Stop => {
                if payload.len() != 8 {
                    return Err(ProtocolError::InvalidPayloadSize(payload.len() as u32));
                }

                let timestamp = u64::from_le_bytes([
                    payload[0], payload[1], payload[2], payload[3], payload[4], payload[5],
                    payload[6], payload[7],
                ]);

                Ok(ProducerMessage::Stop { timestamp })
            }
            ProducerMessageType::EndOfStream => {
                if payload.len() != 8 {
                    return Err(ProtocolError::InvalidPayloadSize(payload.len() as u32));
                }

                let timestamp = u64::from_le_bytes([
                    payload[0], payload[1], payload[2], payload[3], payload[4], payload[5],
                    payload[6], payload[7],
                ]);

                Ok(ProducerMessage::EndOfStream { timestamp })
            }
            ProducerMessageType::Error => {
                let message = String::from_utf8(payload.to_vec())
                    .map_err(|_| ProtocolError::Utf8(std::str::from_utf8(payload).unwrap_err()))?;
                Ok(ProducerMessage::Error { message })
            }
            ProducerMessageType::PlaybackComplete => {
                if payload.len() != 8 {
                    return Err(ProtocolError::InvalidPayloadSize(payload.len() as u32));
                }

                let timestamp = u64::from_le_bytes([
                    payload[0], payload[1], payload[2], payload[3], payload[4], payload[5],
                    payload[6], payload[7],
                ]);

                Ok(ProducerMessage::PlaybackComplete { timestamp })
            }
        }
    }
}

/// Binary protocol connection for consumers
pub struct ConsumerConnection<T: Read + Write> {
    stream: T,
}

impl<T: Read + Write> ConsumerConnection<T> {
    pub fn new(stream: T) -> Self {
        Self { stream }
    }

    /// Read a consumer message from the connection
    pub fn read_message(&mut self) -> Result<ConsumerMessage, ProtocolError> {
        // Read message type (1 byte)
        let mut type_byte = [0u8; 1];
        self.stream.read_exact(&mut type_byte)?;
        let message_type = ConsumerMessageType::try_from(type_byte[0])?;

        // Read payload size (4 bytes, little endian)
        let mut size_bytes = [0u8; 4];
        self.stream.read_exact(&mut size_bytes)?;
        let payload_size = u32::from_le_bytes(size_bytes);

        // Validate payload size (prevent DoS attacks)
        if payload_size > 10 * 1024 * 1024 {
            // 10MB max
            return Err(ProtocolError::InvalidPayloadSize(payload_size));
        }

        // Read binary payload
        let mut payload = vec![0u8; payload_size as usize];
        if payload_size > 0 {
            self.stream.read_exact(&mut payload)?;
        }

        // Deserialize message from binary format
        ConsumerMessage::from_bytes(message_type, &payload)
    }

    /// Write a consumer message to the connection
    pub fn write_message(&mut self, message: &ConsumerMessage) -> Result<(), ProtocolError> {
        let bytes = message.to_bytes()?;
        self.stream.write_all(&bytes)?;
        Ok(())
    }
}

/// Binary protocol connection for producers
pub struct ProducerConnection<T: Read + Write> {
    stream: T,
}

impl<T: Read + Write> ProducerConnection<T> {
    pub fn new(stream: T) -> Self {
        Self { stream }
    }

    /// Read a producer message from the connection
    pub fn read_message(&mut self) -> Result<ProducerMessage, ProtocolError> {
        // Read message type (1 byte)
        let mut type_byte = [0u8; 1];
        self.stream.read_exact(&mut type_byte)?;
        let message_type = ProducerMessageType::try_from(type_byte[0])?;

        // Read payload size (4 bytes, little endian)
        let mut size_bytes = [0u8; 4];
        self.stream.read_exact(&mut size_bytes)?;
        let payload_size = u32::from_le_bytes(size_bytes);

        // Validate payload size (prevent DoS attacks)
        if payload_size > 10 * 1024 * 1024 {
            // 10MB max
            return Err(ProtocolError::InvalidPayloadSize(payload_size));
        }

        // Read binary payload
        let mut payload = vec![0u8; payload_size as usize];
        if payload_size > 0 {
            self.stream.read_exact(&mut payload)?;
        }

        // Deserialize message from binary format
        ProducerMessage::from_bytes(message_type, &payload)
    }

    /// Write a producer message to the connection
    pub fn write_message(&mut self, message: &ProducerMessage) -> Result<(), ProtocolError> {
        let bytes = message.to_bytes()?;
        self.stream.write_all(&bytes)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_consumer_message_binary_serialization() {
        let msg = ConsumerMessage::Error {
            message: "Test error".to_string(),
        };
        let bytes = msg.to_bytes().unwrap();

        // Binary format: [message_type: u8][payload_len: u32][payload_data]
        assert_eq!(bytes[0], ConsumerMessageType::Error as u8);

        // Test round-trip
        let mut cursor = Cursor::new(bytes);
        let mut connection = ConsumerConnection::new(cursor);
        let parsed_msg = connection.read_message().unwrap();

        match parsed_msg {
            ConsumerMessage::Error { message } => {
                assert_eq!(message, "Test error");
            }
            _ => panic!("Expected Error message"),
        }
    }

    #[test]
    fn test_wakeword_detected_binary() {
        let msg = ConsumerMessage::WakewordDetected {
            model: "hey-jarvis".to_string(),
            timestamp: 1234567890,
            spotify_was_paused: true,
        };
        let bytes = msg.to_bytes().unwrap();

        // Should start with WakewordDetected message type
        assert_eq!(bytes[0], ConsumerMessageType::WakewordDetected as u8);

        // Test round-trip
        let mut cursor = Cursor::new(bytes);
        let mut connection = ConsumerConnection::new(cursor);
        let parsed_msg = connection.read_message().unwrap();

        match parsed_msg {
            ConsumerMessage::WakewordDetected {
                model,
                timestamp,
                spotify_was_paused,
            } => {
                assert_eq!(model, "hey-jarvis");
                assert_eq!(timestamp, 1234567890);
                assert_eq!(spotify_was_paused, true);
            }
            _ => panic!("Expected WakewordDetected message"),
        }
    }

    #[test]
    fn test_audio_message_binary() {
        let audio_data = vec![1, 2, 3, 4, 5, 6];
        let msg = ConsumerMessage::Audio {
            data: audio_data.clone(),
            speech_detected: true,
            timestamp: 1234567890,
        };
        let bytes = msg.to_bytes().unwrap();

        // Should start with Audio message type
        assert_eq!(bytes[0], ConsumerMessageType::Audio as u8);

        // Test round-trip
        let mut cursor = Cursor::new(bytes);
        let mut connection = ConsumerConnection::new(cursor);
        let parsed_msg = connection.read_message().unwrap();

        match parsed_msg {
            ConsumerMessage::Audio {
                data,
                speech_detected,
                timestamp,
            } => {
                assert_eq!(data, audio_data);
                assert_eq!(speech_detected, true);
                assert_eq!(timestamp, 1234567890);
            }
            _ => panic!("Expected Audio message"),
        }
    }

    #[test]
    fn test_producer_message_binary() {
        let audio_data = vec![7, 8, 9, 10];
        let msg = ProducerMessage::Play {
            data: audio_data.clone(),
        };
        let bytes = msg.to_bytes().unwrap();

        // Should start with Play message type
        assert_eq!(bytes[0], ProducerMessageType::Play as u8);

        // Test round-trip
        let mut cursor = Cursor::new(bytes);
        let mut connection = ProducerConnection::new(cursor);
        let parsed_msg = connection.read_message().unwrap();

        match parsed_msg {
            ProducerMessage::Play { data } => {
                assert_eq!(data, audio_data);
            }
            _ => panic!("Expected Play message"),
        }
    }

    #[test]
    fn test_producer_stop_binary() {
        let msg = ProducerMessage::Stop {
            timestamp: 9876543210,
        };
        let bytes = msg.to_bytes().unwrap();

        // Should start with Stop message type
        assert_eq!(bytes[0], ProducerMessageType::Stop as u8);

        // Test round-trip
        let mut cursor = Cursor::new(bytes);
        let mut connection = ProducerConnection::new(cursor);
        let parsed_msg = connection.read_message().unwrap();

        match parsed_msg {
            ProducerMessage::Stop { timestamp } => {
                assert_eq!(timestamp, 9876543210);
            }
            _ => panic!("Expected Stop message"),
        }
    }

    #[test]
    fn test_message_type_conversions() {
        // Test ConsumerMessageType conversions
        assert_eq!(
            ConsumerMessageType::try_from(0x11).unwrap(),
            ConsumerMessageType::Error
        );
        assert_eq!(
            ConsumerMessageType::try_from(0x12).unwrap(),
            ConsumerMessageType::Audio
        );
        assert_eq!(
            ConsumerMessageType::try_from(0x15).unwrap(),
            ConsumerMessageType::WakewordDetected
        );
        assert!(ConsumerMessageType::try_from(0xFF).is_err());

        // Test ProducerMessageType conversions
        assert_eq!(
            ProducerMessageType::try_from(0x20).unwrap(),
            ProducerMessageType::Play
        );
        assert_eq!(
            ProducerMessageType::try_from(0x21).unwrap(),
            ProducerMessageType::Stop
        );
        assert_eq!(
            ProducerMessageType::try_from(0x22).unwrap(),
            ProducerMessageType::EndOfStream
        );
        assert_eq!(
            ProducerMessageType::try_from(0x31).unwrap(),
            ProducerMessageType::Error
        );
        assert_eq!(
            ProducerMessageType::try_from(0x32).unwrap(),
            ProducerMessageType::PlaybackComplete
        );
        assert!(ProducerMessageType::try_from(0xFF).is_err());
    }

    #[test]
    fn test_producer_end_of_stream_binary() {
        let msg = ProducerMessage::EndOfStream {
            timestamp: 1234567890,
        };
        let bytes = msg.to_bytes().unwrap();

        // Should start with EndOfStream message type
        assert_eq!(bytes[0], ProducerMessageType::EndOfStream as u8);

        // Test round-trip
        let mut cursor = Cursor::new(bytes);
        let mut connection = ProducerConnection::new(cursor);
        let parsed_msg = connection.read_message().unwrap();

        match parsed_msg {
            ProducerMessage::EndOfStream { timestamp } => {
                assert_eq!(timestamp, 1234567890);
            }
            _ => panic!("Expected EndOfStream message"),
        }
    }

    #[test]
    fn test_producer_playback_complete_binary() {
        let msg = ProducerMessage::PlaybackComplete {
            timestamp: 9876543210,
        };
        let bytes = msg.to_bytes().unwrap();

        // Should start with PlaybackComplete message type
        assert_eq!(bytes[0], ProducerMessageType::PlaybackComplete as u8);

        // Test round-trip
        let mut cursor = Cursor::new(bytes);
        let mut connection = ProducerConnection::new(cursor);
        let parsed_msg = connection.read_message().unwrap();

        match parsed_msg {
            ProducerMessage::PlaybackComplete { timestamp } => {
                assert_eq!(timestamp, 9876543210);
            }
            _ => panic!("Expected PlaybackComplete message"),
        }
    }
}
