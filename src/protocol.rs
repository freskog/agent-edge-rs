use std::io::{Read, Write};
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

/// Consumer message types (Port 8080)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ConsumerMessageType {
    // Client → Audio Crate
    Subscribe = 0x01,

    // Audio Crate → Client
    Connected = 0x10,
    Error = 0x11,
    Audio = 0x12,
    WakewordDetected = 0x15,
}

impl TryFrom<u8> for ConsumerMessageType {
    type Error = ProtocolError;

    fn try_from(value: u8) -> Result<Self, <ConsumerMessageType as TryFrom<u8>>::Error> {
        match value {
            0x01 => Ok(ConsumerMessageType::Subscribe),
            0x10 => Ok(ConsumerMessageType::Connected),
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

    // Audio Crate → Client
    Connected = 0x30,
    Error = 0x31,
}

impl TryFrom<u8> for ProducerMessageType {
    type Error = ProtocolError;

    fn try_from(value: u8) -> Result<Self, <ProducerMessageType as TryFrom<u8>>::Error> {
        match value {
            0x20 => Ok(ProducerMessageType::Play),
            0x21 => Ok(ProducerMessageType::Stop),
            0x30 => Ok(ProducerMessageType::Connected),
            0x31 => Ok(ProducerMessageType::Error),
            _ => Err(ProtocolError::InvalidMessageType(value)),
        }
    }
}

/// Consumer protocol messages
#[derive(Debug, Clone)]
pub enum ConsumerMessage {
    // Client → Audio Crate
    Subscribe { id: String },

    // Audio Crate → Client
    Connected,
    Error { message: String },
    Audio { 
        data: Vec<u8>, 
        speech_detected: bool,  // VAD result for this chunk
    },
    WakewordDetected { model: String },
}

/// Producer protocol messages
#[derive(Debug, Clone)]
pub enum ProducerMessage {
    // Client → Audio Crate
    Play { data: Vec<u8> },
    Stop,

    // Audio Crate → Client
    Connected,
    Error { message: String },
}

impl ConsumerMessage {
    /// Serialize message to binary format: [MessageType: u8][PayloadLength: u32][Payload: bytes]
    pub fn to_bytes(&self) -> Result<Vec<u8>, ProtocolError> {
        let mut bytes = Vec::new();

        match self {
            ConsumerMessage::Subscribe { id } => {
                bytes.push(ConsumerMessageType::Subscribe as u8);
                let id_bytes = id.as_bytes();
                bytes.extend_from_slice(&(id_bytes.len() as u32).to_le_bytes());
                bytes.extend_from_slice(id_bytes);
            }
            ConsumerMessage::Connected => {
                bytes.push(ConsumerMessageType::Connected as u8);
                bytes.extend_from_slice(&0u32.to_le_bytes()); // No payload
            }
            ConsumerMessage::Error { message } => {
                bytes.push(ConsumerMessageType::Error as u8);
                let msg_bytes = message.as_bytes();
                bytes.extend_from_slice(&(msg_bytes.len() as u32).to_le_bytes());
                bytes.extend_from_slice(msg_bytes);
            }
            ConsumerMessage::Audio { data, speech_detected } => {
                bytes.push(ConsumerMessageType::Audio as u8);
                // Payload: [speech_detected: u8][data_length: u32][data: bytes]
                let payload_len = 1 + 4 + data.len(); // 1 byte for bool + 4 bytes for length + data
                bytes.extend_from_slice(&(payload_len as u32).to_le_bytes());
                bytes.push(if *speech_detected { 1u8 } else { 0u8 });
                bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
                bytes.extend_from_slice(data);
            }
            ConsumerMessage::WakewordDetected { model } => {
                bytes.push(ConsumerMessageType::WakewordDetected as u8);
                let model_bytes = model.as_bytes();
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
            ConsumerMessageType::Subscribe => {
                let id = String::from_utf8(payload.to_vec())
                    .map_err(|_| ProtocolError::InvalidString)?;
                Ok(ConsumerMessage::Subscribe { id })
            }
            ConsumerMessageType::Connected => Ok(ConsumerMessage::Connected),
            ConsumerMessageType::Error => {
                let message = String::from_utf8(payload.to_vec())
                    .map_err(|_| ProtocolError::InvalidString)?;
                Ok(ConsumerMessage::Error { message })
            }
            ConsumerMessageType::Audio => {
                if payload.len() < 5 {
                    return Err(ProtocolError::InvalidPayloadSize(payload.len() as u32));
                }
                
                // Parse: [speech_detected: u8][data_length: u32][data: bytes]
                let speech_detected = payload[0] != 0;
                let data_length = u32::from_le_bytes([payload[1], payload[2], payload[3], payload[4]]) as usize;
                
                if payload.len() < 5 + data_length {
                    return Err(ProtocolError::InvalidPayloadSize(payload.len() as u32));
                }
                
                let data = payload[5..5 + data_length].to_vec();
                Ok(ConsumerMessage::Audio { data, speech_detected })
            },
            ConsumerMessageType::WakewordDetected => {
                let model = String::from_utf8(payload.to_vec())
                    .map_err(|_| ProtocolError::InvalidString)?;
                Ok(ConsumerMessage::WakewordDetected { model })
            }
        }
    }
}

impl ProducerMessage {
    /// Serialize message to binary format
    pub fn to_bytes(&self) -> Result<Vec<u8>, ProtocolError> {
        let mut bytes = Vec::new();

        match self {
            ProducerMessage::Play { data } => {
                bytes.push(ProducerMessageType::Play as u8);
                bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
                bytes.extend_from_slice(data);
            }
            ProducerMessage::Stop => {
                bytes.push(ProducerMessageType::Stop as u8);
                bytes.extend_from_slice(&0u32.to_le_bytes()); // No payload
            }
            ProducerMessage::Connected => {
                bytes.push(ProducerMessageType::Connected as u8);
                bytes.extend_from_slice(&0u32.to_le_bytes()); // No payload
            }
            ProducerMessage::Error { message } => {
                bytes.push(ProducerMessageType::Error as u8);
                let msg_bytes = message.as_bytes();
                bytes.extend_from_slice(&(msg_bytes.len() as u32).to_le_bytes());
                bytes.extend_from_slice(msg_bytes);
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
            ProducerMessageType::Stop => Ok(ProducerMessage::Stop),
            ProducerMessageType::Connected => Ok(ProducerMessage::Connected),
            ProducerMessageType::Error => {
                let message = String::from_utf8(payload.to_vec())
                    .map_err(|_| ProtocolError::InvalidString)?;
                Ok(ProducerMessage::Error { message })
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

        // Read payload
        let mut payload = vec![0u8; payload_size as usize];
        if payload_size > 0 {
            self.stream.read_exact(&mut payload)?;
        }

        // Deserialize message
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

        // Read payload
        let mut payload = vec![0u8; payload_size as usize];
        if payload_size > 0 {
            self.stream.read_exact(&mut payload)?;
        }

        // Deserialize message
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
    fn test_consumer_message_serialization() {
        let msg = ConsumerMessage::Subscribe {
            id: "test-client".to_string(),
        };
        let bytes = msg.to_bytes().unwrap();

        // Should be: [0x01][11][test-client]
        assert_eq!(bytes[0], 0x01);
        assert_eq!(
            u32::from_le_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]),
            11
        );
        assert_eq!(&bytes[5..], b"test-client");
    }

    #[test]
    fn test_consumer_connection() {
        let msg = ConsumerMessage::Connected;
        let bytes = msg.to_bytes().unwrap();

        let mut connection = ConsumerConnection::new(Cursor::new(bytes));
        let received_msg = connection.read_message().unwrap();

        matches!(received_msg, ConsumerMessage::Connected);
    }

    #[test]
    fn test_audio_message_with_speech_detection() {
        let audio_data = vec![1, 2, 3, 4, 5, 6];
        let msg = ConsumerMessage::Audio {
            data: audio_data.clone(),
            speech_detected: true,
        };
        let bytes = msg.to_bytes().unwrap();

        // Parse the message back
        let message_type = ConsumerMessageType::try_from(bytes[0]).unwrap();
        let payload_len = u32::from_le_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]) as usize;
        let payload = &bytes[5..5 + payload_len];
        
        let parsed_msg = ConsumerMessage::from_bytes(message_type, payload).unwrap();
        
        match parsed_msg {
            ConsumerMessage::Audio { data, speech_detected } => {
                assert_eq!(data, audio_data);
                assert_eq!(speech_detected, true);
            }
            _ => panic!("Expected Audio message"),
        }
    }

    #[test]
    fn test_audio_message_without_speech() {
        let audio_data = vec![7, 8, 9, 10];
        let msg = ConsumerMessage::Audio {
            data: audio_data.clone(),
            speech_detected: false,
        };
        let bytes = msg.to_bytes().unwrap();

        // Parse the message back
        let message_type = ConsumerMessageType::try_from(bytes[0]).unwrap();
        let payload_len = u32::from_le_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]) as usize;
        let payload = &bytes[5..5 + payload_len];
        
        let parsed_msg = ConsumerMessage::from_bytes(message_type, payload).unwrap();
        
        match parsed_msg {
            ConsumerMessage::Audio { data, speech_detected } => {
                assert_eq!(data, audio_data);
                assert_eq!(speech_detected, false);
            }
            _ => panic!("Expected Audio message"),
        }
    }

    #[test]
    fn test_producer_message_serialization() {
        let audio_data = vec![1, 2, 3, 4];
        let msg = ProducerMessage::Play {
            data: audio_data.clone(),
        };
        let bytes = msg.to_bytes().unwrap();

        // Should be: [0x20][4][1,2,3,4]
        assert_eq!(bytes[0], 0x20);
        assert_eq!(
            u32::from_le_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]),
            4
        );
        assert_eq!(&bytes[5..], &audio_data);
    }
}
