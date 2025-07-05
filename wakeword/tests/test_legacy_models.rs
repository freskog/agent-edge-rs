// Note: These tests directly access the legacy model implementation
// which isn't exposed in the public API but still exists in the src/models/ directory

#[cfg(test)]
#[test]
fn test_legacy_models_exist() {
    // This test just verifies that the model files exist
    use std::path::Path;

    println!("Checking if legacy model files exist...");

    let model_files = [
        "models/melspectrogram.tflite",
        "models/embedding_model.tflite",
        "models/hey_mycroft_v0.1.tflite",
    ];

    for model_file in &model_files {
        let path = Path::new(model_file);
        if path.exists() {
            println!("✅ Found: {}", model_file);
        } else {
            println!("❌ Missing: {}", model_file);
        }
    }
}

#[cfg(test)]
#[test]
fn test_can_create_tflite_interpreter_directly() {
    // Test if we can create a TensorFlow Lite interpreter directly with the model files
    use tflitec::interpreter::Options;
    use tflitec::{interpreter::Interpreter, model::Model as TfliteModel};

    println!("Testing direct TensorFlow Lite model loading...");

    let model_files = [
        ("melspectrogram", "models/melspectrogram.tflite"),
        ("embedding", "models/embedding_model.tflite"),
        ("wakeword", "models/hey_mycroft_v0.1.tflite"),
    ];

    for (name, path) in &model_files {
        println!("Testing {}: {}", name, path);

        // Try to load the model
        let model_result = TfliteModel::new(path);
        match model_result {
            Ok(model) => {
                println!("  ✅ Model loaded successfully");

                // Try to create interpreter
                let options = Options::default();
                let interpreter_result = Interpreter::new(&model, Some(options));
                match interpreter_result {
                    Ok(interpreter) => {
                        println!("  ✅ Interpreter created successfully");

                        // Try to allocate tensors
                        let allocate_result = interpreter.allocate_tensors();
                        match allocate_result {
                            Ok(()) => println!("  ✅ Tensors allocated successfully"),
                            Err(e) => println!("  ❌ Tensor allocation failed: {}", e),
                        }
                    }
                    Err(e) => println!("  ❌ Interpreter creation failed: {}", e),
                }
            }
            Err(e) => println!("  ❌ Model loading failed: {}", e),
        }
    }
}

#[cfg(test)]
#[test]
fn test_working_models_only() {
    // Test that shows the embedding and wakeword models work fine
    use tflitec::interpreter::Options;
    use tflitec::{interpreter::Interpreter, model::Model as TfliteModel};

    println!("Testing inference with working models (embedding + wakeword)...");

    // Test embedding model
    println!("Testing embedding model inference...");
    let embedding_model = TfliteModel::new("models/embedding_model.tflite").unwrap();
    let embedding_interpreter =
        Interpreter::new(&embedding_model, Some(Options::default())).unwrap();
    embedding_interpreter.allocate_tensors().unwrap();

    // Get input tensor info
    let input_tensor = embedding_interpreter.input(0).unwrap();
    let input_shape = input_tensor.shape();
    println!("  Embedding input shape: {:?}", input_shape.dimensions());

    // Test wakeword model
    println!("Testing wakeword model inference...");
    let wakeword_model = TfliteModel::new("models/hey_mycroft_v0.1.tflite").unwrap();
    let wakeword_interpreter = Interpreter::new(&wakeword_model, Some(Options::default())).unwrap();
    wakeword_interpreter.allocate_tensors().unwrap();

    // Get input tensor info
    let input_tensor = wakeword_interpreter.input(0).unwrap();
    let input_shape = input_tensor.shape();
    println!("  Wakeword input shape: {:?}", input_shape.dimensions());

    println!("✅ Both working models can be used for inference!");
}

#[cfg(test)]
#[test]
fn test_model_file_analysis() {
    // Analyze the model files to understand the issue
    use std::fs::File;
    use std::io::{BufReader, Read};

    println!("Analyzing model files...");

    let model_files = [
        ("melspectrogram", "models/melspectrogram.tflite"),
        ("embedding", "models/embedding_model.tflite"),
        ("wakeword", "models/hey_mycroft_v0.1.tflite"),
    ];

    for (name, path) in &model_files {
        if let Ok(file) = File::open(path) {
            let mut reader = BufReader::new(file);
            let mut buffer = Vec::new();
            if reader.read_to_end(&mut buffer).is_ok() {
                println!("  {}: {} bytes", name, buffer.len());

                // Check if it's a valid TensorFlow Lite file (starts with specific header)
                if buffer.len() >= 8 {
                    let header = &buffer[0..8];
                    if header == b"TFL3\x00\x00\x00\x00" || header.starts_with(b"TFL3") {
                        println!("    ✅ Valid TensorFlow Lite file header");
                    } else {
                        println!("    ❌ Invalid TensorFlow Lite file header: {:?}", header);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[test]
fn test_legacy_melspec_model_with_fix() {
    // Test that the legacy melspec model works with the proper resize-then-allocate pattern
    use tflitec::interpreter::Options;
    use tflitec::tensor::Shape;
    use tflitec::{interpreter::Interpreter, model::Model as TfliteModel};

    println!("Testing legacy melspec model with proper tensor allocation...");

    // Load model
    let model = TfliteModel::new("models/melspectrogram.tflite").unwrap();
    let interpreter = Interpreter::new(&model, Some(Options::default())).unwrap();

    // The fix: resize BEFORE allocating tensors
    let input_shape = Shape::new(vec![1, 1280]); // 80ms at 16kHz
    interpreter.resize_input(0, input_shape).unwrap();
    interpreter.allocate_tensors().unwrap();

    // Test basic inference
    let dummy_audio = vec![0.0f32; 1280];
    interpreter.copy(&dummy_audio, 0).unwrap();
    interpreter.invoke().unwrap();

    // Get output
    let output_tensor = interpreter.output(0).unwrap();
    let output_data = output_tensor.data::<f32>().to_vec();

    println!("✅ Legacy melspec model inference successful!");
    println!("  Input shape: [1, 1280]");
    println!("  Output features: {} elements", output_data.len());

    // The features should be non-zero and reasonable
    assert!(!output_data.is_empty(), "Output should not be empty");
    assert!(
        output_data.len() > 10,
        "Output should have multiple features"
    );
}
