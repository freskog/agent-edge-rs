//! # Service Protos
//!
//! Shared protobuf definitions for agent-edge-rs services.
//! This crate contains the generated gRPC service definitions and message types.

pub mod audio {
    tonic::include_proto!("audio");
}

// Re-export common types for convenience
pub use audio::*;
