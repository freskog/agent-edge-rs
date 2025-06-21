use tflitec::interpreter::{Interpreter, Options};
use tflitec::model::Model;
use tflitec::tensor::Shape;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Inspecting TensorFlow Lite models...\n");

    // Inspect melspectrogram model
    println!("=== Melspectrogram Model ===");
    let melspec_model = Model::new("models/melspectrogram.tflite")?;
    let melspec_interpreter = Interpreter::new(&melspec_model, Some(Options::default()))?;
    melspec_interpreter.resize_input(0, Shape::new(vec![1, 1280]))?;
    melspec_interpreter.allocate_tensors()?;

    println!("Input tensors:");
    for i in 0..melspec_interpreter.input_tensor_count() {
        let tensor = melspec_interpreter.input(i)?;
        println!("  Input {}: {:?}", i, tensor.shape());
    }

    println!("Output tensors:");
    for i in 0..melspec_interpreter.output_tensor_count() {
        let tensor = melspec_interpreter.output(i)?;
        println!("  Output {}: {:?}", i, tensor.shape());
    }

    // Inspect embedding model
    println!("\n=== Embedding Model ===");
    let embedding_model = Model::new("models/embedding_model.tflite")?;
    let embedding_interpreter = Interpreter::new(&embedding_model, Some(Options::default()))?;

    // Allocate tensors first so we can inspect them
    embedding_interpreter.allocate_tensors()?;

    println!("Input tensors:");
    for i in 0..embedding_interpreter.input_tensor_count() {
        let tensor = embedding_interpreter.input(i)?;
        println!("  Input {}: {:?}", i, tensor.shape());
    }

    println!("Output tensors:");
    for i in 0..embedding_interpreter.output_tensor_count() {
        let tensor = embedding_interpreter.output(i)?;
        println!("  Output {}: {:?}", i, tensor.shape());
    }

    println!("\n=== Wakeword Model ===");
    let wakeword_model = Model::new("models/hey_mycroft_v0.1.tflite")?;
    let wakeword_interpreter = Interpreter::new(&wakeword_model, Some(Options::default()))?;
    wakeword_interpreter.resize_input(0, Shape::new(vec![1, 6144]))?;
    wakeword_interpreter.allocate_tensors()?;

    println!("Input tensors:");
    for i in 0..wakeword_interpreter.input_tensor_count() {
        let tensor = wakeword_interpreter.input(i)?;
        println!("  Input {}: {:?}", i, tensor.shape());
    }

    println!("Output tensors:");
    for i in 0..wakeword_interpreter.output_tensor_count() {
        let tensor = wakeword_interpreter.output(i)?;
        println!("  Output {}: {:?}", i, tensor.shape());
    }

    // Print a summary of the pipeline architecture
    println!("\n=== OpenWakeWord Pipeline Summary ===");
    let melspec_output_size = 1 * 1 * 5 * 32; // [1, 1, 5, 32]
    println!(
        "Stage 1 - Melspectrogram: [1, 1280] → [1, 1, 5, 32] ({} features per chunk)",
        melspec_output_size
    );

    // Get embedding input size from the tensor shape
    let embedding_input_tensor = embedding_interpreter.input(0)?;
    let embedding_input_shape = embedding_input_tensor.shape();
    let embedding_input_size: usize = embedding_input_shape.dimensions().iter().product();

    let embedding_output_tensor = embedding_interpreter.output(0)?;
    let embedding_output_shape = embedding_output_tensor.shape();
    let embedding_output_size: usize = embedding_output_shape.dimensions().iter().product();

    println!(
        "Stage 2 - Embedding: {:?} → {:?} ({} → {} features)",
        embedding_input_shape.dimensions(),
        embedding_output_shape.dimensions(),
        embedding_input_size,
        embedding_output_size
    );

    println!("Stage 3 - Wakeword: [1, 6144] → [4, 1] (6144 → 4 classes)");

    // Calculate how many melspectrogram chunks are needed for the embedding model
    let chunks_needed = (embedding_input_size + melspec_output_size - 1) / melspec_output_size;
    println!("\nPipeline calculations:");
    println!("- Mel features per chunk: {}", melspec_output_size);
    println!("- Embedding input needed: {}", embedding_input_size);
    println!("- Chunks needed for embedding: {}", chunks_needed);
    println!(
        "- Time to accumulate: {:.1}s (at 80ms per chunk)",
        chunks_needed as f32 * 0.08
    );

    Ok(())
}
