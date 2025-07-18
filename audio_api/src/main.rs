use audio_api::audio_sink::{AudioSink, AudioSinkConfig};
use audio_api::audio_source::{AudioCapture, AudioCaptureConfig};
use audio_api::tcp_server::{AudioServer, ServerConfig};
use clap::Parser;
// Audio device listing functionality is now in the audio modules
use log::{info, warn};

#[derive(Parser)]
#[command(name = "audio_api")]
#[command(about = "Audio API server for real-time audio capture and playback")]
#[command(long_about = "
Audio API server that provides TCP services for real-time audio capture and playback.

EXAMPLES:
  # List available audio devices
  audio_api --list-devices
  
  # Start server with specific devices
  audio_api --input-device \"ReSpeaker 4 Mic Array\" --output-device \"Built-in Audio\"
  
  # Bind to different address/port
  audio_api --bind \"0.0.0.0:8080\"
  
  # For ReSpeaker 4-mic array, use channel 0
  audio_api --input-device \"ReSpeaker 4 Mic Array\" --input-channel 0
")]
struct Args {
    /// TCP bind address and port
    #[arg(long, default_value = "127.0.0.1:50051")]
    bind: String,

    /// Maximum number of concurrent connections
    #[arg(long, default_value = "5")]
    max_connections: usize,

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

    /// Show detailed device information
    #[arg(long)]
    verbose_devices: bool,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    let args = Args::parse();

    // If --list-devices is specified, list devices and exit
    if args.list_devices {
        list_audio_devices(args.verbose_devices)?;
        return Ok(());
    }

    // Log platform information
    info!("🎯 Starting audio API server");
    info!("🔊 Input: mono 16kHz s16le (hardware auto-detected)");
    info!("🔊 Output: mono 44.1kHz s16le → stereo hardware format");

    // Create server configuration
    let server_config = ServerConfig {
        bind_address: args.bind.clone(),
        max_connections: args.max_connections,
        audio_sink_config: AudioSinkConfig {
            device_name: args.output_device.clone(),
        },
        audio_capture_config: AudioCaptureConfig {
            device_id: args.input_device.clone(),
            channel: args.input_channel,
        },
    };

    // Log configuration
    info!("🎵 Starting audio service on TCP: {}", args.bind);
    info!("🔗 Max connections: {}", args.max_connections);
    if let Some(ref input_dev) = args.input_device {
        info!("🎤 Using input device: {} (channel {})", input_dev, args.input_channel);
    } else {
        info!("🎤 Using default input device (channel {})", args.input_channel);
    }
    if let Some(ref output_dev) = args.output_device {
        info!("🔊 Using output device: {}", output_dev);
    } else {
        info!("🔊 Using default output device");
    }

    // Create and run the server
    let server = AudioServer::new(server_config)?;
    server.run()?;

    Ok(())
}

/// List available audio devices
fn list_audio_devices(verbose: bool) -> Result<(), Box<dyn std::error::Error>> {
    
    println!("🎤 Available input devices:");
    match AudioCapture::list_devices() {
        Ok(devices) => {
            if devices.is_empty() {
                println!("  No input devices found");
            } else {
                for device in devices {
                    let default_marker = if device.is_default { " (default)" } else { "" };
                    if verbose {
                        println!("  - {} [{}ch]{}", device.name, device.channel_count, default_marker);
                    } else {
                        println!("  - {}{}", device.name, default_marker);
                    }
                }
            }
        }
        Err(e) => {
            warn!("Failed to list input devices: {}", e);
        }
    }

    println!("\n🔊 Available output devices:");
    match AudioSink::list_devices() {
        Ok(devices) => {
            if devices.is_empty() {
                println!("  No output devices found");
            } else {
                for device in devices {
                    let default_marker = if device.is_default { " (default)" } else { "" };
                    if verbose {
                        println!("  - {} [{}ch]{}", device.name, device.channel_count, default_marker);
                    } else {
                        println!("  - {}{}", device.name, default_marker);
                    }
                }
            }
        }
        Err(e) => {
            warn!("Failed to list output devices: {}", e);
        }
    }

    if !verbose {
        println!("\n💡 Use --verbose-devices for detailed information");
    }

    println!("\n📖 Usage examples:");
    println!("  # List devices:");
    println!("  audio_api --list-devices");
    println!("  audio_api --list-devices --verbose-devices");
    println!("\n  # Use specific devices:");
    println!("  audio_api --input-device \"ReSpeaker 4 Mic Array\" --input-channel 0");
    println!("  audio_api --output-device \"Built-in Audio\" --input-device \"USB Audio\"");
    println!("\n  # Custom bind address:");
    println!("  audio_api --bind \"0.0.0.0:8080\" --max-connections 10");

    Ok(())
}
