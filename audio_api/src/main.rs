use audio_api::audio_sink::CpalConfig;
use audio_api::audio_source::AudioCaptureConfig;
use audio_api::tonic::service::{run_server, run_server_unix, AudioServiceImpl};
use clap::Parser;
use cpal::traits::{DeviceTrait, HostTrait};
use log::{info, warn};

#[derive(Parser)]
#[command(name = "audio_api")]
#[command(about = "Audio API server for real-time audio capture and playback")]
#[command(long_about = "
Audio API server that provides gRPC services for real-time audio capture and playback.

EXAMPLES:
  # List available audio devices
  audio_api --list-devices
  
  # Start server with specific devices
  audio_api --input-device \"ReSpeaker 4 Mic Array\" --input-channel 0
  
  # Use Unix socket for better performance
  audio_api --unix --input-device \"USB Audio\" --output-device \"Built-in Audio\"
  
  # For ReSpeaker 4-mic array, use channel 0-5 (or 0-3 depending on firmware)
  audio_api --unix --input-device \"ReSpeaker 4 Mic Array\" --input-channel 1
")]
struct Args {
    /// Use Unix domain socket instead of TCP
    #[arg(long, default_value = "false")]
    unix: bool,

    /// Socket path for Unix domain socket (default: /tmp/audio_api.sock)
    #[arg(long, default_value = "/tmp/audio_api.sock")]
    socket_path: String,

    /// TCP address and port (default: 127.0.0.1:50051)
    #[arg(long, default_value = "127.0.0.1:50051")]
    tcp_addr: String,

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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    let args = Args::parse();

    // If --list-devices is specified, list devices and exit
    if args.list_devices {
        list_audio_devices(args.verbose_devices)?;
        return Ok(());
    }

    // Create audio sink configuration
    let sink_config = CpalConfig {
        device_name: args.output_device.clone(),
        ..Default::default()
    };

    // Create audio capture configuration
    let capture_config = AudioCaptureConfig {
        device_id: args.input_device.clone(),
        channel: args.input_channel,
        ..Default::default()
    };

    // Create service with custom configurations
    let service = AudioServiceImpl::with_configs(sink_config, capture_config)?;

    if args.unix {
        info!(
            "🎵 Starting audio service on Unix domain socket: {}",
            args.socket_path
        );
        if let Some(ref input_dev) = args.input_device {
            info!(
                "🎤 Using input device: {} (channel {})",
                input_dev, args.input_channel
            );
        }
        if let Some(ref output_dev) = args.output_device {
            info!("🔊 Using output device: {}", output_dev);
        }
        run_server_unix(&args.socket_path, service).await?;
    } else {
        let addr: std::net::SocketAddr = args.tcp_addr.parse()?;
        info!("🎵 Starting audio service on TCP: {}", addr);
        if let Some(ref input_dev) = args.input_device {
            info!(
                "🎤 Using input device: {} (channel {})",
                input_dev, args.input_channel
            );
        }
        if let Some(ref output_dev) = args.output_device {
            info!("🔊 Using output device: {}", output_dev);
        }
        warn!("💡 Tip: Use --unix for better performance with Unix domain sockets");
        run_server(addr, service).await?;
    }

    Ok(())
}

fn list_audio_devices(verbose: bool) -> Result<(), Box<dyn std::error::Error>> {
    let host = cpal::default_host();

    println!("🎤 Available Input Devices:");
    println!("==========================");

    // List input devices using CPAL directly for better compatibility
    match host.input_devices() {
        Ok(devices) => {
            let default_input = host.default_input_device();
            let mut device_count = 0;

            for device in devices {
                if let Ok(name) = device.name() {
                    // Only list devices that actually support input
                    if let Ok(config) = device.default_input_config() {
                        device_count += 1;
                        let is_default = default_input
                            .as_ref()
                            .map(|d| d.name().unwrap_or_default() == name)
                            .unwrap_or(false);
                        let default_marker = if is_default { " (default)" } else { "" };

                        println!(
                            "  {}. {}{} - {} channels",
                            device_count,
                            name,
                            default_marker,
                            config.channels()
                        );

                        if verbose {
                            println!("     Format: {:?}", config.sample_format());
                            println!("     Sample rate: {}Hz", config.sample_rate().0);
                            println!("     Channels: {}", config.channels());

                            if let Ok(configs) = device.supported_input_configs() {
                                println!("     Supported configs:");
                                for config in configs {
                                    println!(
                                        "       {:?}, {} channels, {}-{} Hz",
                                        config.sample_format(),
                                        config.channels(),
                                        config.min_sample_rate().0,
                                        config.max_sample_rate().0
                                    );
                                }
                            }
                        }
                    }
                }
            }

            if device_count == 0 {
                println!("  No input devices found");
            }
        }
        Err(e) => {
            println!("  Error listing input devices: {}", e);
        }
    }

    println!("\n🔊 Available Output Devices:");
    println!("============================");

    // List output devices
    match host.output_devices() {
        Ok(devices) => {
            let default_output = host.default_output_device();
            let mut device_count = 0;

            for device in devices {
                device_count += 1;
                let name = device.name().unwrap_or_else(|e| format!("<error: {}>", e));
                let is_default = default_output
                    .as_ref()
                    .map(|d| d.name().unwrap_or_default() == name)
                    .unwrap_or(false);
                let default_marker = if is_default { " (default)" } else { "" };

                println!("  {}. {}{}", device_count, name, default_marker);

                if verbose {
                    if let Ok(config) = device.default_output_config() {
                        println!("     Format: {:?}", config.sample_format());
                        println!("     Sample rate: {}Hz", config.sample_rate().0);
                        println!("     Channels: {}", config.channels());
                    }
                    if let Ok(configs) = device.supported_output_configs() {
                        println!("     Supported configs:");
                        for config in configs {
                            println!(
                                "       {:?}, {}-{} channels, {}-{} Hz",
                                config.sample_format(),
                                config.channels(),
                                config.channels(),
                                config.min_sample_rate().0,
                                config.max_sample_rate().0
                            );
                        }
                    }
                }
            }

            if device_count == 0 {
                println!("  No output devices found");
            }
        }
        Err(e) => {
            println!("  Error listing output devices: {}", e);
        }
    }

    println!("\n💡 Usage Examples:");
    println!("=================");
    println!("  # List devices:");
    println!("  audio_api --list-devices");
    println!("  audio_api --list-devices --verbose-devices");
    println!("\n  # Use specific devices:");
    println!("  audio_api --input-device \"ReSpeaker 4 Mic Array\" --input-channel 0");
    println!("  audio_api --output-device \"Built-in Audio\" --input-device \"USB Audio\"");
    println!("\n  # Use Unix socket (recommended):");
    println!("  audio_api --unix --input-device \"ReSpeaker 4 Mic Array\" --input-channel 1");

    Ok(())
}
