// Test to examine the exact tensor shapes expected by each model
use tflitec::interpreter::Options;
use tflitec::tensor::Shape;
use tflitec::{interpreter::Interpreter, model::Model as TfliteModel};

#[cfg(test)]
#[test]
fn debug_all_model_tensor_shapes() {
    println!("=== DEBUGGING ALL MODEL TENSOR SHAPES ===");

    let model_files = [
        ("melspectrogram", "models/melspectrogram.tflite"),
        ("embedding", "models/embedding_model.tflite"),
        ("wakeword", "models/hey_mycroft_v0.1.tflite"),
    ];

    for (name, path) in &model_files {
        println!("\n📋 Analyzing {}: {}", name, path);

        let model = TfliteModel::new(path).unwrap();
        let interpreter = Interpreter::new(&model, Some(Options::default())).unwrap();

        // For melspectrogram, we need to resize first
        if name == &"melspectrogram" {
            let input_shape = Shape::new(vec![1, 1280]); // 80ms at 16kHz
            interpreter.resize_input(0, input_shape).unwrap();
        }

        interpreter.allocate_tensors().unwrap();

        // Check input tensor
        let input_tensor = interpreter.input(0).unwrap();
        let input_shape = input_tensor.shape();
        let input_elements = input_shape.dimensions().iter().product::<usize>();

        println!("  📥 Input shape: {:?}", input_shape.dimensions());
        println!("  📥 Input elements: {}", input_elements);

        // Check output tensor
        let output_tensor = interpreter.output(0).unwrap();
        let output_shape = output_tensor.shape();
        let output_elements = output_shape.dimensions().iter().product::<usize>();

        println!("  📤 Output shape: {:?}", output_shape.dimensions());
        println!("  📤 Output elements: {}", output_elements);

        // Calculate expected relationships
        match name {
            &"melspectrogram" => {
                println!("  🔍 Expected: 1280 audio samples → ? mel features");
                println!("  🔍 Actual output: {} features", output_elements);
            }
            &"embedding" => {
                println!("  🔍 Expected: 76 mel frames × 32 features = 2432 input elements");
                println!("  🔍 Actual input: {} elements", input_elements);
                println!("  🔍 Expected output: 96 embedding features");
                println!("  🔍 Actual output: {} features", output_elements);
            }
            &"wakeword" => {
                println!("  🔍 Expected: 16 embeddings × 96 features = 1536 input elements");
                println!("  🔍 Actual input: {} elements", input_elements);
                println!("  🔍 Expected output: 1 confidence score");
                println!("  🔍 Actual output: {} scores", output_elements);
            }
            _ => {}
        }
    }

    println!("\n🔄 EXPECTED PIPELINE FLOW:");
    println!("  1. Raw audio (1280 samples) → melspectrogram → ? mel features");
    println!("  2. Mel features (76 frames × 32 = 2432) → embedding → 96 features");
    println!("  3. Embeddings (16 × 96 = 1536) → wakeword → 1 confidence score");
}
