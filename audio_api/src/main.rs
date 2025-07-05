use audio_api::audio_sink::CpalConfig;
use audio_api::tonic::service::{run_server, run_server_unix, AudioServiceImpl};
use clap::Parser;
use log::{info, warn};

#[derive(Parser)]
#[command(name = "audio_api")]
#[command(about = "Audio API server for real-time audio capture and playback")]
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
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    let args = Args::parse();

    let service = AudioServiceImpl::new_with_config(CpalConfig::default())?;

    if args.unix {
        info!(
            "🎵 Starting audio service on Unix domain socket: {}",
            args.socket_path
        );
        run_server_unix(&args.socket_path, service).await?;
    } else {
        let addr: std::net::SocketAddr = args.tcp_addr.parse()?;
        info!("🎵 Starting audio service on TCP: {}", addr);
        warn!("💡 Tip: Use --unix for better performance with Unix domain sockets");
        run_server(addr, service).await?;
    }

    Ok(())
}
