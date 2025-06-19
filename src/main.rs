use anyhow::Result;
use clap::Parser;
use env_logger;
use log::{info, warn};

mod audio;
mod detection;
mod error;
mod models;

use crate::detection::{DetectionPipeline, PipelineConfig};
use crate::models::WakewordConfig;

#[derive(Parser)]
#[command(name = "agent-edge")]
#[command(about = "Wakeword-only edge client for low-powered devices")]
struct Cli {
    /// Enable verbose logging
    #[arg(short, long)]
    verbose: bool,

    /// Path to the wakeword model file
    #[arg(long, default_value = "models/hey_mycroft_v0.1.tflite")]
    wakeword_model: String,

    /// Path to the melspectrogram model file
    #[arg(long, default_value = "models/melspectrogram.tflite")]
    melspec_model: String,

    /// Confidence threshold for wakeword detection
    #[arg(long, default_value = "0.8")]
    threshold: f32,

    /// Enable debug mode for detailed logging
    #[arg(long)]
    debug: bool,

    /// Run basic functionality test
    #[arg(long)]
    test: bool,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    // Initialize logging
    if cli.verbose {
        env_logger::Builder::from_default_env()
            .filter_level(log::LevelFilter::Debug)
            .init();
    } else {
        env_logger::init();
    }

    info!("Starting Edge AI Wakeword Detection Agent");

    // Create wakeword configuration
    let wakeword_config = WakewordConfig {
        wakeword_model_path: cli.wakeword_model,
        melspec_model_path: cli.melspec_model,
        confidence_threshold: cli.threshold,
        sample_rate: 16000,
        chunk_size: 1280, // 80ms at 16kHz
    };

    // Create pipeline configuration
    let pipeline_config = PipelineConfig {
        wakeword_config,
        debug_mode: cli.debug,
    };

    if cli.test {
        return run_test(pipeline_config);
    }

    // Initialize the detection pipeline
    let mut pipeline = DetectionPipeline::new(pipeline_config)?;

    info!("Pipeline initialized successfully");
    info!(
        "Chunk size: {} samples ({} ms)",
        pipeline.chunk_size_samples(),
        pipeline.chunk_duration_ms()
    );
    info!("Detection threshold: {:.2}", pipeline.get_threshold());

    // TODO: Integrate with audio capture system
    // For now, we'll run a simple test to verify everything works
    run_audio_test(&mut pipeline)?;

    info!("Edge AI Agent shutting down");
    Ok(())
}

/// Run a basic test with synthetic audio
fn run_test(config: PipelineConfig) -> Result<(), Box<dyn std::error::Error>> {
    info!("Running basic functionality test...");

    let mut pipeline = DetectionPipeline::new(config)?;

    // Generate test audio: sine wave at 440Hz
    let chunk_size = pipeline.chunk_size_samples();
    let sample_rate = 16000.0;
    let frequency = 440.0; // A4 note

    let test_audio: Vec<f32> = (0..chunk_size)
        .map(|i| {
            let t = i as f32 / sample_rate;
            (2.0 * std::f32::consts::PI * frequency * t).sin() * 0.3
        })
        .collect();

    info!("Generated {} samples of test audio", test_audio.len());

    // Process the test audio
    match pipeline.process_chunk(&test_audio) {
        Ok(detection) => {
            info!("✅ Test processing successful!");
            info!("  - Confidence: {:.3}", detection.confidence);
            info!("  - Detected: {}", detection.detected);

            let stats = pipeline.stats();
            info!("  - Processing time: {:.2}ms", stats.avg_processing_time_ms);
        }
        Err(e) => {
            warn!("❌ Test processing failed: {}", e);
            return Err(e.into());
        }
    }

    info!("✅ Basic functionality test completed successfully");
    Ok(())
}

/// Run a basic audio processing test
fn run_audio_test(pipeline: &mut DetectionPipeline) -> Result<(), Box<dyn std::error::Error>> {
    info!("Running audio processing test with multiple chunks...");

    let chunk_size = pipeline.chunk_size_samples();
    let sample_rate = 16000.0;

    // Generate different test patterns
    let test_patterns = vec![
        ("silence", vec![0.0f32; chunk_size]),
        (
            "sine_440hz",
            (0..chunk_size)
                .map(|i| {
                    let t = i as f32 / sample_rate;
                    (2.0 * std::f32::consts::PI * 440.0 * t).sin() * 0.3
                })
                .collect(),
        ),
        (
            "sine_880hz",
            (0..chunk_size)
                .map(|i| {
                    let t = i as f32 / sample_rate;
                    (2.0 * std::f32::consts::PI * 880.0 * t).sin() * 0.3
                })
                .collect(),
        ),
    ];

    for (name, audio_data) in test_patterns {
        info!("Processing {} pattern...", name);

        match pipeline.process_chunk(&audio_data) {
            Ok(detection) => {
                info!("  - Confidence: {:.3}", detection.confidence);
                info!(
                    "  - Detected: {}",
                    if detection.detected { "YES" } else { "no" }
                );
            }
            Err(e) => {
                warn!("  - Error processing {}: {}", name, e);
            }
        }
    }

    let stats = pipeline.stats();
    info!("Processing complete:");
    info!("  - Chunks processed: {}", stats.chunks_processed);
    info!("  - Detections: {}", stats.detections_count);
    info!(
        "  - Average processing time: {:.2}ms",
        stats.avg_processing_time_ms
    );

    Ok(())
}
