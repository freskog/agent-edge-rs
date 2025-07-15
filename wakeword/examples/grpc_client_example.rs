//! # gRPC Client Example
//!
//! This example demonstrates how to use the wake word detection gRPC client
//! to connect to the audio API and detect wake words from live audio.
//!
//! ## Prerequisites
//!
//! 1. Start the audio API server:
//!    ```bash
//!    cd audio_api && cargo run -- --socket /tmp/audio_api.sock
//!    ```
//!
//! 2. Run this example:
//!    ```bash
//!    cd wakeword && cargo run --example grpc_client_example
//!    ```
//!
//! ## Usage
//!
//! The example will:
//! - Connect to the audio API via Unix socket
//! - Subscribe to live audio streams
//! - Process audio chunks and detect wake words
//! - Print detection results to console

use log::{error, info};
use std::env;
use wakeword::grpc_client;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    env_logger::init();

    info!("🚀 Starting Wake Word Detection gRPC Client Example");

    // Configuration
    let socket_path =
        env::var("AUDIO_API_SOCKET").unwrap_or_else(|_| "/tmp/audio_api.sock".to_string());
    let model_names = vec!["hey_mycroft".to_string(), "hey_jarvis".to_string()];
    let detection_threshold = 0.5;

    info!("📋 Configuration:");
    info!("   Socket: {}", socket_path);
    info!("   Models: {:?}", model_names);
    info!("   Threshold: {}", detection_threshold);

    // Check if socket exists
    if !std::path::Path::new(&socket_path).exists() {
        error!("❌ Audio API socket not found: {}", socket_path);
        error!("   Please start the audio API server first:");
        error!("   cd audio_api && cargo run -- --socket {}", socket_path);
        std::process::exit(1);
    }

    info!("🔌 Connecting to audio API...");

    // Start wake word detection
    match grpc_client::start_wakeword_detection(&socket_path, model_names, detection_threshold)
        .await
    {
        Ok(()) => {
            info!("✅ Wake word detection completed successfully");
        }
        Err(e) => {
            error!("❌ Wake word detection failed: {}", e);

            // Provide helpful error messages
            let error_msg = e.to_string();
            if error_msg.contains("Connection refused")
                || error_msg.contains("No such file or directory")
            {
                error!("💡 Troubleshooting:");
                error!("   1. Make sure the audio API server is running");
                error!("   2. Check the socket path is correct");
                error!("   3. Verify the socket file exists and is accessible");
            } else if error_msg.contains("models") {
                error!("💡 Troubleshooting:");
                error!("   1. Make sure wake word model files are available");
                error!("   2. Check the models directory exists");
                error!("   3. Verify model files are not corrupted");
            } else if error_msg.contains("TensorFlow") {
                error!("💡 Troubleshooting:");
                error!("   1. Make sure TensorFlow Lite is properly installed");
                error!("   2. Check XNNPACK is working correctly");
                error!("   3. Verify the platform is supported");
            }

            std::process::exit(1);
        }
    }

    Ok(())
}
