use audio::{MockAudioServer, MockServerConfig};
use clap::Parser;
use log::info;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "mock_audio_server")]
#[command(about = "Mock audio server that plays existing wave files for testing")]
struct Args {
    /// Address to bind the TCP server to
    #[arg(short, long, default_value = "127.0.0.1:8080")]
    address: String,

    /// Wave file to play (should be 16kHz mono s16le)
    #[arg(short, long, default_value = "../tests/data/hey_mycroft_test.wav")]
    file: PathBuf,

    /// Loop the audio file continuously
    #[arg(long)]
    loop_audio: bool,

    /// Silence duration after file ends before looping (in seconds)
    #[arg(long, default_value = "2.0")]
    silence_duration: f32,

    /// Playback speed multiplier (1.0 = real time, 2.0 = 2x speed)
    #[arg(long, default_value = "1.0")]
    speed: f32,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let args = Args::parse();
    info!("🚀 Starting mock audio server with args: {:?}", args);

    let config = MockServerConfig {
        audio_file: args.file,
        bind_address: args.address,
        loop_audio: args.loop_audio,
        silence_duration: args.silence_duration,
        speed: args.speed,
    };

    let mut server = MockAudioServer::new(config)?;

    // Handle Ctrl+C gracefully
    let should_stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let should_stop_clone = should_stop.clone();
    ctrlc::set_handler(move || {
        info!("🛑 Received Ctrl+C, shutting down...");
        should_stop_clone.store(true, std::sync::atomic::Ordering::Relaxed);
    })?;

    let port = server.start()?;
    info!("🎵 Mock audio server started on port {}", port);

    // Wait for shutdown signal
    while !should_stop.load(std::sync::atomic::Ordering::Relaxed) {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    server.stop();
    info!("🛑 Mock audio server stopped");

    Ok(())
}
