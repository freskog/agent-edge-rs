use clap::{Parser, Subcommand};
use std::path::PathBuf;

use wakeword::*;

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

        /// Model to use
        #[arg(short, long, default_value = "hey_mycroft")]
        model: String,
    },
    /// Test XNNPACK functionality
    TestXnnpack {
        /// Model to use for testing
        #[arg(short, long, default_value = "embedding_model")]
        model: String,
    },
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let cli = Cli::parse();

    match &cli.command {
        Commands::Test { input, model } => {
            println!("Testing audio file: {}", input.display());
            println!("Using model: {}", model);

            // Load the model
            let model_path = format!("models/{}.tflite", model);
            let mut wakeword_model = Model::new(&model_path)?;

            // Load audio file
            let mut reader = hound::WavReader::open(input)?;
            let samples: Vec<f32> = reader
                .samples::<i16>()
                .map(|s| s.unwrap() as f32 / i16::MAX as f32)
                .collect();

            println!("Loaded {} samples", samples.len());

            // Process audio
            let predictions = wakeword_model.predict(&samples)?;

            println!("Predictions: {:?}", predictions);

            for (model_name, confidence) in predictions {
                if confidence > 0.5 {
                    println!(
                        "🎯 Detected '{}' with confidence {:.3}",
                        model_name, confidence
                    );
                }
            }
        }
        Commands::TestXnnpack { model } => {
            println!("=== Testing XNNPACK with {} ===", model);

            // Test both CPU and XNNPACK
            test_xnnpack_performance(model)?;
        }
    }

    Ok(())
}

fn test_xnnpack_performance(model_name: &str) -> Result<(), Box<dyn std::error::Error>> {
    use std::time::Instant;
    use tflitec::interpreter::{Interpreter, Options};
    use tflitec::model::Model as TfliteModel;

    let model_path = format!("models/{}.tflite", model_name);
    let model = TfliteModel::new(&model_path)?;

    // Test 1: CPU-only
    println!("🔧 Testing CPU-only inference...");
    let mut cpu_options = Options::default();
    cpu_options.thread_count = 1;
    #[cfg(feature = "xnnpack")]
    {
        cpu_options.is_xnnpack_enabled = false;
    }

    let cpu_interpreter = Interpreter::new(&model, Some(cpu_options))?;
    cpu_interpreter.allocate_tensors()?;

    let input_tensor = cpu_interpreter.input(0)?;
    let input_size = input_tensor.shape().dimensions().iter().product::<usize>();
    let dummy_input: Vec<f32> = (0..input_size).map(|i| (i as f32) * 0.01).collect();

    // Warm up and benchmark CPU
    let mut cpu_times = Vec::new();
    for _ in 0..10 {
        let start = Instant::now();
        cpu_interpreter.copy(&dummy_input, 0)?;
        cpu_interpreter.invoke()?;
        let _output = cpu_interpreter.output(0)?;
        cpu_times.push(start.elapsed());
    }

    let cpu_avg = cpu_times.iter().sum::<std::time::Duration>() / cpu_times.len() as u32;
    println!("  CPU-only average time: {:?}", cpu_avg);
    println!(
        "  CPU-only inferences/sec: {:.2}",
        1.0 / cpu_avg.as_secs_f64()
    );

    // Test 2: XNNPACK
    println!("🚀 Testing XNNPACK-accelerated inference...");
    let mut xnnpack_options = Options::default();
    xnnpack_options.thread_count = 1;
    #[cfg(feature = "xnnpack")]
    {
        xnnpack_options.is_xnnpack_enabled = true;
    }

    let xnnpack_interpreter = Interpreter::new(&model, Some(xnnpack_options))?;
    xnnpack_interpreter.allocate_tensors()?;

    // Warm up and benchmark XNNPACK
    let mut xnnpack_times = Vec::new();
    for _ in 0..10 {
        let start = Instant::now();
        xnnpack_interpreter.copy(&dummy_input, 0)?;
        xnnpack_interpreter.invoke()?;
        let _output = xnnpack_interpreter.output(0)?;
        xnnpack_times.push(start.elapsed());
    }

    let xnnpack_avg =
        xnnpack_times.iter().sum::<std::time::Duration>() / xnnpack_times.len() as u32;
    println!("  XNNPACK average time: {:?}", xnnpack_avg);
    println!(
        "  XNNPACK inferences/sec: {:.2}",
        1.0 / xnnpack_avg.as_secs_f64()
    );

    // Compare performance
    let speedup = cpu_avg.as_secs_f64() / xnnpack_avg.as_secs_f64();
    println!("🎯 XNNPACK speedup: {:.2}x faster than CPU", speedup);

    Ok(())
}
