//! Simple example of using the wakeword_protocol client
//!
//! This example demonstrates how to connect to a wakeword server and listen for events.
//!
//! First, start the wakeword service with event server:
//! ```bash
//! cd wakeword && cargo run -- --server 127.0.0.1:50051 --wakeword-server 127.0.0.1:50052 --models hey_mycroft
//! ```
//!
//! Then run this example:
//! ```bash
//! cd wakeword_protocol && cargo run --example simple_client
//! ```

use log::{error, info};
use wakeword_protocol::WakewordClient;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    info!("🚀 Starting wakeword client example");

    // Connect to the wakeword server
    let mut client = WakewordClient::connect("192.168.8.132:50052")?;
    info!("✅ Connected to wakeword server");

    // Subscribe to wakeword events
    match client.subscribe_wakeword()? {
        wakeword_protocol::SubscribeResult::Success => {
            info!("🔔 Successfully subscribed to wakeword events");
        }
        wakeword_protocol::SubscribeResult::AlreadySubscribed => {
            info!("ℹ️ Already subscribed to wakeword events");
        }
        wakeword_protocol::SubscribeResult::Error(msg) => {
            error!("❌ Failed to subscribe: {}", msg);
            return Err(msg.into());
        }
    }

    // Listen for wakeword events
    info!("👂 Listening for wakeword events... (Press Ctrl+C to stop)");

    client.listen_for_events(|event| {
        println!(
            "🎯 WAKE WORD DETECTED: '{}' with confidence {:.3} (client: {})",
            event.model_name, event.confidence, event.client_id
        );

        // You could trigger actions here based on the detected wake word
        match event.model_name.as_str() {
            "hey_mycroft" => println!("  → Mycroft wake word detected!"),
            "hey_jarvis" => println!("  → Jarvis wake word detected!"),
            "alexa" => println!("  → Alexa wake word detected!"),
            _ => println!("  → Unknown wake word: {}", event.model_name),
        }
    })?;

    Ok(())
}
