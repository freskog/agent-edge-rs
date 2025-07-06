use tflitec::model::Model;
use wakeword::xnnpack_fix;

fn main() {
    println!("=== XNNPACK Debug Binary (FIXED) ===");

    // Enable debug logging
    std::env::set_var("RUST_LOG", "debug");
    env_logger::init();

    let model_path = "models/embedding_model.tflite";
    println!("Loading model: {}", model_path);

    let model = Model::new(model_path).expect("Failed to load model");
    println!("✅ Model loaded successfully");

    println!("Creating XNNPACK options with fix...");
    let xnnpack_options = xnnpack_fix::create_xnnpack_options(1);
    println!(
        "✅ XNNPACK options created with fix (threads: {})",
        xnnpack_options.num_threads
    );

    println!("Creating interpreter with FIXED XNNPACK...");
    println!("✅ Using our working XNNPACK fix - no segfault expected!");

    // This should NOT segfault anymore
    let interpreter = xnnpack_fix::create_interpreter_with_xnnpack_safe(&model, 1)
        .expect("Failed to create interpreter");
    println!("✅ Interpreter created successfully with fix!");

    println!("Allocating tensors...");
    interpreter
        .allocate_tensors()
        .expect("Failed to allocate tensors");
    println!("✅ Tensors allocated");

    println!("🎉 FIXED XNNPACK is working correctly without segfault!");
}
