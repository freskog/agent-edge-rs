use audio::audio_sink::AudioSinkConfig;
use audio::audio_source::AudioCaptureConfig;
use audio::consumer_server::{ConsumerServer, ConsumerServerConfig};
use audio::producer_server::{ProducerServer, ProducerServerConfig};
// Import wakeword configuration
use audio::wakeword_vad::VadConfig;
use clap::Parser;
use log::{error, info};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

#[derive(Parser)]
#[command(name = "audio_service")]
#[command(about = "Binary Audio Service - Consumer and Producer TCP servers")]
#[command(long_about = "
Binary Audio Service providing two TCP interfaces:

CONSUMER INTERFACE (Port 8080):
  - Single consumer can subscribe for audio stream + events
  - Receives 16kHz s16le mono audio chunks
  - Receives events: SpeechStarted, SpeechStopped, WakewordDetected

PRODUCER INTERFACE (Port 8081):
  - Single producer can send audio for playback
  - Sends 44.1kHz mono s16le audio chunks
  - Can send Stop command to clear audio queue

EXAMPLES:
  # Start service with default ports
  audio_service
  
  # Start with custom ports  
  audio_service --consumer-port 9080 --producer-port 9081
  
  # List available audio devices
  audio_service --list-devices
  
  # Use specific audio devices
  audio_service --input-device \"ReSpeaker 4 Mic Array\" --output-device \"Built-in Audio\"
")]
struct Args {
    /// Consumer server bind address (for audio streaming)
    #[arg(long, default_value = "127.0.0.1:8080")]
    consumer_bind: String,

    /// Producer server bind address (for audio playback)
    #[arg(long, default_value = "127.0.0.1:8081")]
    producer_bind: String,

    /// List available audio devices and exit
    #[arg(long)]
    list_devices: bool,

    /// Input device name for audio capture
    #[arg(long)]
    input_device: Option<String>,

    /// Output device name for audio playback
    #[arg(long)]
    output_device: Option<String>,

    /// Input channel to capture from (0-based index)
    #[arg(long, default_value = "0")]
    input_channel: u32,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let args = Args::parse();

    if args.list_devices {
        list_audio_devices();
        return Ok(());
    }

    info!("🚀 Starting Binary Audio Service with Wakeword Detection");
    info!("🎯 Consumer server: {}", args.consumer_bind);
    info!("🔊 Producer server: {}", args.producer_bind);

    // Prepare configurations
    let consumer_config = ConsumerServerConfig {
        bind_address: args.consumer_bind,
        audio_capture_config: AudioCaptureConfig {
            device_id: args.input_device.clone(),
            channel: args.input_channel,
        },
        wakeword_models: vec!["hey_mycroft".to_string()], // Default wakeword model
        detection_threshold: 0.5,                         // Default detection threshold
        vad_config: VadConfig::default(),                 // Default VAD configuration
    };

    let producer_config = ProducerServerConfig {
        bind_address: args.producer_bind,
        audio_sink_config: AudioSinkConfig {
            device_name: args.output_device.clone(),
        },
    };

    // Create servers
    let consumer_server = ConsumerServer::new(consumer_config);
    let producer_server = ProducerServer::new(producer_config);

    // Shared shutdown signal
    let shutdown = Arc::new(AtomicBool::new(false));

    // Signal handling
    let shutdown_signal = Arc::clone(&shutdown);
    ctrlc::set_handler(move || {
        info!("🛑 Received shutdown signal");
        shutdown_signal.store(true, Ordering::SeqCst);
    })
    .expect("Error setting Ctrl-C handler");

    // Start consumer server in thread
    let consumer_shutdown = Arc::clone(&shutdown);
    let consumer_handle = thread::spawn(move || {
        if let Err(e) = consumer_server.run() {
            error!("❌ Consumer server error: {}", e);
        }
        consumer_shutdown.store(true, Ordering::SeqCst);
    });

    // Start producer server in thread
    let producer_shutdown = Arc::clone(&shutdown);
    let producer_handle = thread::spawn(move || {
        if let Err(e) = producer_server.run() {
            error!("❌ Producer server error: {}", e);
        }
        producer_shutdown.store(true, Ordering::SeqCst);
    });

    // Wait for shutdown signal
    while !shutdown.load(Ordering::SeqCst) {
        thread::sleep(std::time::Duration::from_millis(100));
    }

    info!("🛑 Shutting down servers...");

    // Wait for threads to finish
    if let Err(e) = consumer_handle.join() {
        error!("❌ Consumer server thread panic: {:?}", e);
    }

    if let Err(e) = producer_handle.join() {
        error!("❌ Producer server thread panic: {:?}", e);
    }

    info!("✅ Binary Audio Service shutdown complete");
    Ok(())
}

fn list_audio_devices() {
    use cpal::traits::{DeviceTrait, HostTrait};

    info!("🎵 Available Audio Devices:");

    let host = cpal::default_host();

    // Input devices
    info!("📥 INPUT DEVICES:");
    match host.input_devices() {
        Ok(devices) => {
            for (i, device) in devices.enumerate() {
                match device.name() {
                    Ok(name) => {
                        let is_default = host
                            .default_input_device()
                            .map_or(false, |d| d.name().unwrap_or_default() == name);
                        let marker = if is_default { " (default)" } else { "" };
                        info!("  {}. {}{}", i + 1, name, marker);
                    }
                    Err(e) => info!("  {}. [Error getting name: {}]", i + 1, e),
                }
            }
        }
        Err(e) => error!("❌ Failed to enumerate input devices: {}", e),
    }

    // Output devices
    info!("📤 OUTPUT DEVICES:");
    match host.output_devices() {
        Ok(devices) => {
            for (i, device) in devices.enumerate() {
                match device.name() {
                    Ok(name) => {
                        let is_default = host
                            .default_output_device()
                            .map_or(false, |d| d.name().unwrap_or_default() == name);
                        let marker = if is_default { " (default)" } else { "" };
                        info!("  {}. {}{}", i + 1, name, marker);
                    }
                    Err(e) => info!("  {}. [Error getting name: {}]", i + 1, e),
                }
            }
        }
        Err(e) => error!("❌ Failed to enumerate output devices: {}", e),
    }
}
