//! # Audio Protocol
//!
//! TCP protocol and client for communicating with the audio_api server.
//!
//! This crate provides:
//! - Low-level protocol definitions (messages, serialization)
//! - High-level client for easy usage
//!
//! ## Example Usage
//!
//! ```rust,no_run
//! use audio_protocol::client::AudioClient;
//! use std::time::Duration;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Connect to the audio server
//! let mut client = AudioClient::connect("127.0.0.1:50051")?;
//!
//! // Subscribe to audio capture
//! client.subscribe_audio()?;
//!
//! // Read some audio chunks
//! for _ in 0..10 {
//!     if let Some(chunk) = client.read_audio_chunk()? {
//!         println!("Received {} bytes", chunk.size_bytes());
//!     }
//! }
//!
//! // Play the audio back
//! // (implementation depends on your audio processing pipeline)
//! # Ok(())
//! # }
//! ```

pub mod client;
pub mod protocol;

// Re-export commonly used types
pub use client::{
    AbortResult, AudioChunk, AudioClient, EndStreamResult, PlayResult, UnsubscribeResult,
};
pub use protocol::{Connection, Message, MessageType, ProtocolError};
