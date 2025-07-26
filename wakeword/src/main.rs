use clap::Parser;
use log::{error, info};

#[derive(Parser)]
#[command(name = "wakeword")]
#[command(about = "OpenWakeWord detection using TensorFlow Lite")]
struct Args {
    /// TCP server address for audio_api connection
    #[arg(short, long, default_value = "127.0.0.1:50051")]
    server: String,

    /// Models to use (comma-separated)
    #[arg(short, long, default_value = "hey_mycroft")]
    models: String,

    /// Detection threshold
    #[arg(short, long, default_value = "0.5")]
    threshold: f32,

    /// Wakeword event server address (optional - if provided, will serve wakeword events)
    #[arg(long)]
    wakeword_server: Option<String>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let args = Args::parse();

    info!("👂 Starting live wake word detection");
    info!("🔌 Connecting to audio server at: {}", &args.server);

    let model_names = parse_model_list(&args.models);
    info!("📋 Using models: {:?}", &model_names);
    info!("🎯 Detection threshold: {}", args.threshold);

    let result = if let Some(wakeword_server_addr) = &args.wakeword_server {
        info!(
            "🌐 Starting wakeword event server at: {}",
            wakeword_server_addr
        );
        wakeword::tcp_client::start_wakeword_detection_with_server(
            &args.server,
            wakeword_server_addr,
            model_names,
            args.threshold,
        )
    } else {
        info!("ℹ️ Running without wakeword event server (use --wakeword-server to enable)");
        wakeword::tcp_client::start_wakeword_detection(&args.server, model_names, args.threshold)
    };

    if let Err(e) = result {
        error!("❌ TCP client failed: {}", e);
        std::process::exit(1);
    }

    Ok(())
}

fn parse_model_list(models: &str) -> Vec<String> {
    models
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}
