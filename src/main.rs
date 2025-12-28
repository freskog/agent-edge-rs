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
  - Sends 48kHz mono s16le audio chunks
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

    /// Wakeword detection channel (0-based index, defaults to same as input_channel)
    #[arg(long)]
    wakeword_channel: Option<u32>,

    /// Spotify player name for playerctl (e.g., "spotify", "spotifyd")
    /// If not specified, will auto-detect any available music player
    #[arg(long)]
    spotify_player: Option<String>,
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
    let wakeword_channel = args.wakeword_channel.unwrap_or(args.input_channel);

    let consumer_config = ConsumerServerConfig {
        bind_address: args.consumer_bind,
        audio_capture_config: AudioCaptureConfig {
            device_id: args.input_device.clone(),
            channel: args.input_channel,
        },
        wakeword_channel, // Wakeword detection channel
        wakeword_models: vec!["hey_mycroft".to_string()], // Default wakeword model
        detection_threshold: 0.5, // Default detection threshold
        vad_config: VadConfig::default(), // Default VAD configuration
        spotify_player: args.spotify_player.clone(), // Spotify player for playerctl
    };

    let producer_config = ProducerServerConfig {
        bind_address: args.producer_bind,
        audio_sink_config: AudioSinkConfig {
            device_name: args.output_device.clone(),
        },
    };

    // Create barge-in channel for automatic server-side interruption
    // When consumer detects wakeword during playback, producer aborts immediately
    // Bounded channel (size 1) so old barge-in signals are dropped if not consumed
    // This prevents stale wakeword detections from aborting future audio
    let (barge_in_tx, barge_in_rx) = crossbeam::channel::bounded(1);

    // Create servers
    let mut consumer_server = ConsumerServer::new(consumer_config);
    let mut producer_server = ProducerServer::new(producer_config);

    // Connect barge-in channel
    consumer_server.set_barge_in_sender(barge_in_tx);
    producer_server.set_barge_in_receiver(barge_in_rx);

    // Pre-initialize audio sink to prevent audio loss on first connection
    if let Err(e) = producer_server.initialize_sink() {
        error!("Failed to pre-initialize audio sink: {}", e);
        error!("Audio playback may have issues on first connection");
    }

    // Wrap in Arc for sharing
    let consumer_server = Arc::new(consumer_server);
    let producer_server = Arc::new(producer_server);

    // Shared shutdown signal
    let shutdown = Arc::new(AtomicBool::new(false));

    // Clone server references for shutdown
    let consumer_server_ref = Arc::clone(&consumer_server);
    let producer_server_ref = Arc::clone(&producer_server);

    // Signal handling
    let shutdown_signal = Arc::clone(&shutdown);
    ctrlc::set_handler(move || {
        info!("🛑 Received shutdown signal");
        shutdown_signal.store(true, Ordering::SeqCst);

        // Stop servers explicitly
        consumer_server_ref.stop();
        producer_server_ref.stop();
    })
    .expect("Error setting Ctrl-C handler");

    // Start consumer server in thread
    let consumer_server_thread = Arc::clone(&consumer_server);
    let consumer_shutdown = Arc::clone(&shutdown);
    let consumer_handle = thread::spawn(move || {
        if let Err(e) = consumer_server_thread.run() {
            error!("❌ Consumer server error: {}", e);
        }
        consumer_shutdown.store(true, Ordering::SeqCst);
    });

    // Start producer server in thread
    let producer_server_thread = Arc::clone(&producer_server);
    let producer_shutdown = Arc::clone(&shutdown);
    let producer_handle = thread::spawn(move || {
        if let Err(e) = producer_server_thread.run() {
            error!("❌ Producer server error: {}", e);
        }
        producer_shutdown.store(true, Ordering::SeqCst);
    });

    // Wait for shutdown signal
    while !shutdown.load(Ordering::SeqCst) {
        thread::sleep(std::time::Duration::from_millis(100));
    }

    info!("🛑 Shutting down servers...");

    // Stop servers explicitly (same as Ctrl-C handler)
    consumer_server.stop();
    producer_server.stop();

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
