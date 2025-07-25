use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "check_xnnpack_compat")]
#[command(about = "Check if TFLite models are compatible with XNNPACK")]
struct Args {
    /// Path to the TFLite model file
    #[arg(short, long)]
    model: PathBuf,
}

fn main() -> Result<()> {
    let args = Args::parse();

    println!("🔍 Analyzing model: {}", args.model.display());

    // Try to create interpreter with XNNPACK
    let options = tflitec::interpreter::Options {
        thread_count: 1,
        is_xnnpack_enabled: true,
    };

    let model = tflitec::model::Model::from_file(&args.model)?;

    println!("📋 Creating interpreter with XNNPACK...");

    match tflitec::interpreter::Interpreter::with_model_and_options(&model, &options) {
        Ok(mut interpreter) => {
            println!("✅ XNNPACK delegate created successfully");

            println!("🔧 Trying to allocate tensors...");
            match interpreter.allocate_tensors() {
                Ok(_) => {
                    println!("✅ Model is XNNPACK compatible!");
                    println!("🎯 This model should work with XNNPACK acceleration");
                }
                Err(e) => {
                    println!("❌ Tensor allocation failed: {}", e);
                    println!("🔍 This model has XNNPACK incompatible operations");
                    println!("💡 Suggestion: Use CPU-only mode for this model");
                }
            }
        }
        Err(e) => {
            println!("❌ Failed to create interpreter: {}", e);
        }
    }

    // Try without XNNPACK for comparison
    println!("\n🔄 Testing without XNNPACK for comparison...");
    let cpu_options = tflitec::interpreter::Options {
        thread_count: 1,
        is_xnnpack_enabled: false,
    };

    match tflitec::interpreter::Interpreter::with_model_and_options(&model, &cpu_options) {
        Ok(mut interpreter) => match interpreter.allocate_tensors() {
            Ok(_) => {
                println!("✅ CPU-only mode works fine");
            }
            Err(e) => {
                println!("❌ Even CPU-only failed: {}", e);
            }
        },
        Err(e) => {
            println!("❌ CPU interpreter creation failed: {}", e);
        }
    }

    Ok(())
}
