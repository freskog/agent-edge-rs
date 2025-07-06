use clap::{Parser, Subcommand};
use log::{error, info};
use std::path::PathBuf;

use wakeword::{grpc_client, Model};

#[derive(Parser)]
#[command(name = "wakeword")]
#[command(about = "OpenWakeWord detection using TensorFlow Lite")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Test audio file processing
    Test {
        /// Path to WAV file
        #[arg(short, long)]
        input: PathBuf,

        /// Models to use (comma-separated)
        #[arg(short, long, default_value = "hey_mycroft")]
        models: String,

        /// Detection threshold
        #[arg(short, long, default_value = "0.5")]
        threshold: f32,
    },
    /// Connect to audio_api via gRPC and detect wake words from live audio stream
    Listen {
        /// Unix socket path for audio_api connection
        #[arg(short, long, default_value = "/tmp/audio_api.sock")]
        socket: String,

        /// Models to use (comma-separated)
        #[arg(short, long, default_value = "hey_mycroft")]
        models: String,

        /// Detection threshold
        #[arg(short, long, default_value = "0.5")]
        threshold: f32,
    },
    /// Test XNNPACK performance
    Benchmark {
        /// Model to use for testing
        #[arg(short, long, default_value = "hey_mycroft")]
        model: String,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let cli = Cli::parse();

    match &cli.command {
        Commands::Test {
            input,
            models,
            threshold,
        } => {
            info!("🧪 Testing audio file: {}", input.display());

            let model_names = parse_model_list(models);
            info!("📋 Using models: {:?}", model_names);
            info!("🎯 Detection threshold: {}", threshold);

            // Initialize the model
            let mut model = Model::new(
                model_names.clone(),
                vec![], // Empty metadata
                0.5,    // VAD threshold
                0.5,    // Custom verifier threshold
            )?;

            // Load audio file
            let mut reader = hound::WavReader::open(input)?;
            let spec = reader.spec();

            info!(
                "📊 Audio format: {}Hz, {} channels, {} bits",
                spec.sample_rate, spec.channels, spec.bits_per_sample
            );

            // Read samples as i16
            let samples: Vec<i16> = reader.samples::<i16>().map(|s| s.unwrap()).collect();

            info!(
                "📦 Loaded {} samples ({:.2}s)",
                samples.len(),
                samples.len() as f32 / spec.sample_rate as f32
            );

            // Process audio in chunks
            const CHUNK_SIZE: usize = 16000; // 1 second at 16kHz
            let mut detected_any = false;

            for (chunk_idx, chunk) in samples.chunks(CHUNK_SIZE).enumerate() {
                let chunk_start_time =
                    chunk_idx as f32 * CHUNK_SIZE as f32 / spec.sample_rate as f32;

                match model.predict(chunk, None, 1.0) {
                    Ok(predictions) => {
                        for (model_name, confidence) in predictions {
                            if confidence > *threshold {
                                info!(
                                    "🎯 DETECTED '{}' at {:.1}s with confidence {:.3}",
                                    model_name, chunk_start_time, confidence
                                );
                                detected_any = true;
                            } else if confidence > 0.1 {
                                info!(
                                    "🔍 Low confidence '{}' at {:.1}s: {:.3}",
                                    model_name, chunk_start_time, confidence
                                );
                            }
                        }
                    }
                    Err(e) => {
                        error!("❌ Detection failed at {:.1}s: {}", chunk_start_time, e);
                    }
                }
            }

            if !detected_any {
                info!("🔇 No wake words detected above threshold {}", threshold);
            }
        }

        Commands::Listen {
            socket,
            models,
            threshold,
        } => {
            info!("👂 Starting live wake word detection");
            info!("🔌 Connecting to audio_api at: {}", socket);

            let model_names = parse_model_list(models);
            info!("📋 Using models: {:?}", model_names);
            info!("🎯 Detection threshold: {}", threshold);

            // Start the gRPC client
            if let Err(e) =
                grpc_client::start_wakeword_detection(socket, model_names, *threshold).await
            {
                error!("❌ gRPC client failed: {}", e);
                std::process::exit(1);
            }
        }

        Commands::Benchmark { model } => {
            info!("⚡ Running performance benchmark");

            // Use the existing benchmark test from lib.rs but as a function
            // For now, just run a simple test
            let model_names = vec![format!("{}_v0.1", model)];
            let mut wakeword_model = Model::new(model_names, vec![], 0.5, 0.5)?;

            // Generate dummy audio
            let dummy_audio: Vec<i16> = (0..16000)
                .map(|i| ((i as f32 * 0.001).sin() * 1000.0) as i16)
                .collect();

            // Benchmark
            let start = std::time::Instant::now();
            let iterations = 100;

            for _ in 0..iterations {
                let _ = wakeword_model.predict(&dummy_audio, None, 1.0)?;
            }

            let elapsed = start.elapsed();
            let avg_ms = elapsed.as_nanos() as f64 / iterations as f64 / 1_000_000.0;

            info!("📊 Benchmark Results:");
            info!("   Average inference time: {:.3} ms", avg_ms);
            info!("   Inferences per second: {:.1}", 1000.0 / avg_ms);
        }
    }

    Ok(())
}

/// Parse comma-separated model list
fn parse_model_list(models_str: &str) -> Vec<String> {
    models_str
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}
