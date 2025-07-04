use clap::Parser;

#[derive(Parser)]
#[command(name = "wakeword")]
#[command(about = "Wakeword detection using TensorFlow Lite with XNNPACK")]
struct Args {
    /// Path to the wakeword model
    #[arg(short, long, default_value = "models/wakeword.tflite")]
    model: String,

    /// Detection threshold
    #[arg(short, long, default_value_t = 0.5)]
    threshold: f32,

    /// Audio device name (optional)
    #[arg(short, long)]
    device: Option<String>,
}

fn load_env() {
    // TODO: Implement local env loading or use dotenv directly
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load environment variables
    load_env();

    // Initialize logging
    env_logger::init();

    // Parse command line arguments
    let args = Args::parse();

    log::info!("Starting wakeword detection...");
    log::info!("Model: {}", args.model);
    log::info!("Threshold: {}", args.threshold);

    // TODO: Implement wakeword detection logic
    // This will use TensorFlow Lite with XNNPACK

    Ok(())
}
