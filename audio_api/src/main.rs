use audio_api::{
    audio_sink::{AudioSink, CpalConfig, CpalSink},
    audio_source::{AudioCapture, AudioCaptureConfig},
    error::AudioError,
    error::Result as AudioResult,
};
use clap::{Parser, Subcommand};
use log::{error, info, warn};
use std::time::Duration;
use tokio::time::sleep;

#[derive(Parser)]
#[command(name = "audio-api")]
#[command(about = "Audio API service for agent-edge-rs")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// List available audio devices
    ListDevices,
    /// Test audio capture from a device
    Capture {
        /// Device ID to capture from (use 'list-devices' to see available devices)
        #[arg(long)]
        device_id: Option<String>,
        /// Duration to capture in seconds
        #[arg(long, default_value = "5")]
        duration: u64,
    },
    /// Test audio playback
    Playback {
        /// Device name to play to (use 'list-devices' to see available devices)
        #[arg(long)]
        device_name: Option<String>,
        /// Duration to play test tone in seconds
        #[arg(long, default_value = "3")]
        duration: u64,
    },
    /// Start the gRPC server (placeholder for future implementation)
    Serve {
        /// Port to listen on
        #[arg(long, default_value = "50051")]
        port: u16,
    },
}

#[tokio::main]
async fn main() -> AudioResult<()> {
    // Initialize logging
    env_logger::init();

    info!("🎵 Starting Audio API service");

    let cli = Cli::parse();

    match cli.command {
        Commands::ListDevices => {
            list_devices().await?;
        }
        Commands::Capture {
            device_id,
            duration,
        } => {
            test_capture(device_id, duration).await?;
        }
        Commands::Playback {
            device_name,
            duration,
        } => {
            test_playback(device_name, duration).await?;
        }
        Commands::Serve { port } => {
            info!("🚀 Starting gRPC server on port {}", port);
            // TODO: Implement gRPC server
            warn!("gRPC server not yet implemented - this is a placeholder");
            // Keep the process running for now
            loop {
                sleep(Duration::from_secs(1)).await;
            }
        }
    }

    Ok(())
}

async fn list_devices() -> AudioResult<()> {
    info!("📋 Listing available audio devices...");

    match AudioCapture::list_devices() {
        Ok(devices) => {
            if devices.is_empty() {
                warn!("No audio devices found");
                return Ok(());
            }

            info!("Found {} audio device(s):", devices.len());
            for (i, device) in devices.iter().enumerate() {
                let default_marker = if device.is_default { " (default)" } else { "" };
                info!(
                    "  {}. {} [{}]{} - {} channels",
                    i + 1,
                    device.name,
                    device.id,
                    default_marker,
                    device.channel_count
                );
            }
        }
        Err(e) => {
            error!("Failed to list devices: {}", e);
            return Err(AudioError::AudioCapture(e));
        }
    }

    Ok(())
}

async fn test_capture(device_id: Option<String>, duration: u64) -> AudioResult<()> {
    info!("🎤 Testing audio capture for {} seconds...", duration);

    let config = AudioCaptureConfig {
        device_id,
        ..Default::default()
    };

    let (tx, mut rx) = tokio::sync::mpsc::channel(128);
    let mut chunk_count = 0;
    let start_time = std::time::Instant::now();

    let _audio_capture = AudioCapture::new(config, tx)?;

    info!("🎤 Audio capture started successfully");
    info!("Press Ctrl+C to stop early");

    // Receive chunks for the specified duration
    let timeout = tokio::time::sleep(Duration::from_secs(duration));
    tokio::pin!(timeout);

    loop {
        tokio::select! {
            _ = &mut timeout => break,
            chunk = rx.recv() => {
                match chunk {
                    Some(_chunk) => {
                        chunk_count += 1;
                        if chunk_count % 50 == 0 {  // Log every 50 chunks (4 seconds)
                            info!("Captured {} chunks", chunk_count);
                        }
                    }
                    None => break, // Channel closed
                }
            }
        }
    }

    let elapsed = start_time.elapsed();
    info!(
        "🎤 Capture completed: {} chunks in {:?}",
        chunk_count, elapsed
    );

    Ok(())
}

async fn test_playback(device_name: Option<String>, duration: u64) -> AudioResult<()> {
    info!("🔊 Testing audio playback for {} seconds...", duration);

    let config = CpalConfig {
        device_name,
        ..Default::default()
    };

    let sink = CpalSink::new(config)?;
    info!("🔊 Audio sink created successfully");

    // Generate a simple test tone (440Hz sine wave)
    let sample_rate = 16000;
    let frequency = 440.0;
    let amplitude = 0.3;

    let num_samples = (sample_rate as f64 * duration as f64) as usize;
    let mut test_tone = Vec::with_capacity(num_samples);

    for i in 0..num_samples {
        let t = i as f64 / sample_rate as f64;
        let sample = (2.0 * std::f64::consts::PI * frequency * t).sin() * amplitude;
        test_tone.push(sample as f32);
    }

    // Convert to bytes (f32 samples)
    let audio_data: Vec<u8> = test_tone
        .iter()
        .flat_map(|&sample| sample.to_le_bytes())
        .collect();

    info!("🔊 Playing test tone...");
    sink.write(&audio_data).await?;

    // Wait for playback to complete
    sleep(Duration::from_secs(duration)).await;

    info!("🔊 Playback test completed");

    Ok(())
}
