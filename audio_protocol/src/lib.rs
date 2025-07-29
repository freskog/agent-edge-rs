//! # Audio Protocol
//!
//! TCP protocol and client for communicating with the audio_api server.
//!
//! This crate provides:
//! - Low-level protocol definitions (messages, serialization)
//! - High-level client for easy usage
//! - Buffered client for continuous streaming with internal buffering
//!
//! ## Example Usage
//!
//! ### Basic Client
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
//! # Ok(())
//! # }
//! ```
//!
//! ### Buffered Client (Recommended)
//! ```rust,no_run
//! use audio_protocol::client::BufferedAudioClient;
//! use std::time::Duration;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Connect with 2-second internal buffer
//! let client = BufferedAudioClient::connect_default("127.0.0.1:50051")?;
//!
//! // Read chunks continuously - no data loss!
//! for _ in 0..100 {
//!     if let Some(chunk) = client.read_chunk_timeout(Duration::from_millis(100)) {
//!         println!("Received {} bytes", chunk.size_bytes());
//!     }
//! }
//!
//! // Check buffer health
//! if let Some(stats) = client.get_stats() {
//!     if !stats.is_healthy() {
//!         println!("Warning: Audio buffer is unhealthy!");
//!         stats.log_status();
//!     }
//! }
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
