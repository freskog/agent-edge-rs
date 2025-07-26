//! # Wakeword Protocol
//!
//! TCP protocol and client for communicating with wakeword detection services.
//!
//! This crate provides:
//! - Low-level protocol definitions (wakeword events, message serialization)
//! - High-level client for easy subscription to wakeword events
//!
//! ## Example Usage
//!
//! ```rust,no_run
//! use wakeword_protocol::client::WakewordClient;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Connect to the wakeword server
//! let mut client = WakewordClient::connect("127.0.0.1:50052")?;
//!
//! // Subscribe to wakeword events
//! client.subscribe_wakeword()?;
//!
//! // Listen for wake word detections
//! while let Some(event) = client.read_wakeword_event()? {
//!     println!("Wake word '{}' detected with confidence: {:.3}",
//!              event.model_name, event.confidence);
//! }
//! # Ok(())
//! # }
//! ```

pub mod client;
pub mod protocol;

// Re-export commonly used types
pub use client::{SubscribeResult, WakewordClient};
pub use protocol::{Connection, Message, MessageType, ProtocolError, WakewordEvent};
