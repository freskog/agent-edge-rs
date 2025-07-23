//! # TCP Client Example
//!
//! This example demonstrates how to use the wake word detection TCP client
//! to connect to the audio API and detect wake words from live audio.
//!
//! ## Prerequisites
//!
//! 1. Start the audio API server:
//!    ```bash
//!    cd audio_api && cargo run -- --address 127.0.0.1:50051
//!    ```
//!
//! 2. Run this example:
//!    ```bash
//!    cd wakeword && cargo run --example tcp_client_example
//!    ```
//!
//! ## Usage
//!
//! The example will:
//! - Connect to the audio API via TCP
//! - Subscribe to live audio streams
//! - Process audio chunks and detect wake words
//! - Print detection results to console

use log::{error, info};
use std::env;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    env_logger::init();

    info!("🚀 Starting Wake Word Detection TCP Client Example");

    // Configuration
    let server_address =
        env::var("AUDIO_SERVER_ADDRESS").unwrap_or_else(|_| "127.0.0.1:50051".to_string());
    let model_names = vec!["hey_mycroft".to_string(), "hey_jarvis".to_string()];
    let detection_threshold = 0.5;

    info!("📋 Configuration:");
    info!("   Server: {}", server_address);
    info!("   Models: {:?}", model_names);
    info!("   Threshold: {}", detection_threshold);

    info!("🔌 Connecting to audio server...");

    // Start wake word detection (now synchronous!)
    match wakeword::tcp_client::start_wakeword_detection(
        &server_address,
        model_names,
        detection_threshold,
    ) {
        Ok(()) => {
            info!("✅ Wake word detection completed successfully");
        }
        Err(e) => {
            error!("❌ Wake word detection failed: {}", e);

            // Provide helpful error messages
            let error_msg = e.to_string();
            if error_msg.contains("Connection refused") || error_msg.contains("connect") {
                error!("💡 Troubleshooting:");
                error!("   1. Make sure the audio API server is running");
                error!(
                    "   2. Check the server address is correct: {}",
                    server_address
                );
                error!("   3. Verify the server is listening on the specified port");
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
