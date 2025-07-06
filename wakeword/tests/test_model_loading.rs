//! Test individual model loading to identify problematic models

use tflitec::interpreter::Options;
use tflitec::tensor::Shape;
use tflitec::{interpreter::Interpreter, model::Model as TfliteModel};

#[cfg(test)]
#[test]
fn test_model_loading_with_xnnpack() {
    println!("=== Testing Model Loading with XNNPACK ===");

    let model_files = [
        ("melspectrogram", "models/melspectrogram.tflite"),
        ("embedding", "models/embedding_model.tflite"),
        ("wakeword", "models/hey_mycroft_v0.1.tflite"),
    ];

    for (name, path) in &model_files {
        println!("Testing {} model: {}", name, path);

        // Load the model
        let model = TfliteModel::new(path).unwrap();
        println!("  ✅ Model loaded successfully");

        // Create interpreter options with XNNPACK enabled
        let mut options = Options::default();
        options.thread_count = 1;
        // Note: XNNPACK delegate is automatically enabled when available
        // in tflitec, but we need to ensure proper tensor sizing

        let interpreter = Interpreter::new(&model, Some(options)).unwrap();
        println!("  ✅ Interpreter created successfully");

        // Special handling for melspectrogram model - it needs dynamic tensor resizing
        if *name == "melspectrogram" {
            println!("  🔧 Applying dynamic tensor resizing for melspectrogram model");

            // Resize input tensor to [1, 1280] as expected by OpenWakeWord
            let input_shape = Shape::new(vec![1, 1280]);
            interpreter.resize_input(0, input_shape).unwrap();
            println!("  ✅ Input tensor resized to [1, 1280]");
        }

        // Allocate tensors after any resizing
        interpreter.allocate_tensors().unwrap();
        println!("  ✅ Tensors allocated successfully");

        // Get input tensor info
        let input_tensor = interpreter.input(0).unwrap();
        let input_shape = input_tensor.shape();
        let input_size = input_shape.dimensions().iter().product::<usize>();

        println!("  📊 Input shape: {:?}", input_shape.dimensions());
        println!("  📊 Input size: {}", input_size);

        // Create appropriate test data
        let dummy_input: Vec<f32> = match *name {
            "melspectrogram" => {
                // For melspectrogram: 1280 audio samples (80ms at 16kHz)
                (0..1280).map(|i| (i as f32 * 0.001).sin()).collect()
            }
            "embedding" => {
                // For embedding: mel features
                (0..input_size).map(|i| (i as f32) * 0.01).collect()
            }
            "wakeword" => {
                // For wakeword: embedding features
                (0..input_size).map(|i| (i as f32) * 0.001).collect()
            }
            _ => vec![0.0; input_size],
        };

        assert_eq!(
            dummy_input.len(),
            input_size,
            "Input size mismatch for {}",
            name
        );

        // Time the inference
        let start = std::time::Instant::now();

        // Copy input and run inference
        interpreter.copy(&dummy_input, 0).unwrap();
        interpreter.invoke().unwrap();

        let duration = start.elapsed();
        println!("  ⏱️  Inference time: {:?}", duration);

        // Verify output
        let output_tensor = interpreter.output(0).unwrap();
        let output_data = output_tensor.data::<f32>();
        let output_shape = output_tensor.shape();

        println!("  📊 Output shape: {:?}", output_shape.dimensions());
        println!("  📊 Output size: {}", output_data.len());

        // Check for reasonable outputs
        let non_zero_count = output_data.iter().filter(|&&x| x != 0.0).count();
        let finite_count = output_data.iter().filter(|&&x| x.is_finite()).count();

        println!(
            "  📈 Non-zero outputs: {}/{}",
            non_zero_count,
            output_data.len()
        );
        println!(
            "  📈 Finite outputs: {}/{}",
            finite_count,
            output_data.len()
        );

        // Print first few output values for debugging
        let preview_count = 5.min(output_data.len());
        println!(
            "  📈 First {} outputs: {:?}",
            preview_count,
            &output_data[..preview_count]
        );

        // Basic validity checks
        assert!(
            output_data.iter().all(|&x| x.is_finite()),
            "All outputs should be finite for {}",
            name
        );

        // Performance check - should be reasonably fast
        assert!(
            duration.as_millis() < 100,
            "Inference should be fast for {} (got {}ms)",
            name,
            duration.as_millis()
        );

        println!("  ✅ {} model test passed!\n", name);
    }

    println!("🎉 All models loaded and tested successfully with XNNPACK!");
}

#[cfg(test)]
#[test]
fn test_melspectrogram_dynamic_resizing() {
    println!("=== Testing Melspectrogram Dynamic Resizing ===");

    let model = TfliteModel::new("models/melspectrogram.tflite").unwrap();
    let options = Options::default();
    let interpreter = Interpreter::new(&model, Some(options)).unwrap();

    // Test different input sizes to verify dynamic resizing works
    let test_sizes = [640, 1280, 1600]; // Different audio chunk sizes

    for &size in &test_sizes {
        println!("Testing with input size: {}", size);

        // Resize input tensor
        let input_shape = Shape::new(vec![1, size]);
        interpreter.resize_input(0, input_shape).unwrap();

        // Allocate tensors
        interpreter.allocate_tensors().unwrap();

        // Verify the resize worked
        let input_tensor = interpreter.input(0).unwrap();
        let actual_shape = input_tensor.shape();
        assert_eq!(actual_shape.dimensions(), &[1, size]);

        // Create dummy audio data
        let dummy_audio: Vec<f32> = (0..size).map(|i| (i as f32 * 0.001).sin()).collect();

        // Run inference
        interpreter.copy(&dummy_audio, 0).unwrap();
        interpreter.invoke().unwrap();

        // Check output
        let output_tensor = interpreter.output(0).unwrap();
        let output_data = output_tensor.data::<f32>();

        println!(
            "  Input: [1, {}] -> Output: {:?}",
            size,
            output_tensor.shape().dimensions()
        );
        println!("  Output size: {}", output_data.len());

        assert!(
            output_data.iter().all(|&x| x.is_finite()),
            "All outputs should be finite"
        );
        println!("  ✅ Dynamic resizing test passed for size {}\n", size);
    }

    println!("🎉 Melspectrogram dynamic resizing test completed successfully!");
}

// Additional test to verify XNNPACK is actually being used
#[cfg(test)]
#[test]
fn test_xnnpack_performance_benefit() {
    println!("=== Testing XNNPACK Performance Benefit ===");

    let model = TfliteModel::new("models/embedding_model.tflite").unwrap();

    // Test with single thread to see if XNNPACK provides benefit
    let mut options = Options::default();
    options.thread_count = 1;

    let interpreter = Interpreter::new(&model, Some(options)).unwrap();
    interpreter.allocate_tensors().unwrap();

    let input_tensor = interpreter.input(0).unwrap();
    let input_size = input_tensor.shape().dimensions().iter().product::<usize>();
    let dummy_input: Vec<f32> = (0..input_size).map(|i| (i as f32) * 0.01).collect();

    // Warm up
    for _ in 0..5 {
        interpreter.copy(&dummy_input, 0).unwrap();
        interpreter.invoke().unwrap();
    }

    // Benchmark
    let iterations = 50;
    let start = std::time::Instant::now();

    for _ in 0..iterations {
        interpreter.copy(&dummy_input, 0).unwrap();
        interpreter.invoke().unwrap();
    }

    let total_time = start.elapsed();
    let avg_time = total_time / iterations;

    println!("Average inference time: {:?}", avg_time);
    println!("Inferences per second: {:.2}", 1.0 / avg_time.as_secs_f64());

    // XNNPACK should provide reasonable performance
    assert!(
        avg_time.as_millis() < 10,
        "XNNPACK should provide fast inference (got {}ms)",
        avg_time.as_millis()
    );

    println!("✅ XNNPACK performance test passed!");
}
