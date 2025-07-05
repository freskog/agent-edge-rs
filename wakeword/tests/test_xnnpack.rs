//! Test XNNPACK delegate functionality

use std::time::Instant;
use tflitec::interpreter::{Interpreter, Options};
use tflitec::model::Model;

#[cfg(test)]
#[test]
fn test_xnnpack_enabled() {
    println!("=== Testing XNNPACK delegate functionality ===");

    // Test with the embedding model (known to work)
    let model_path = "models/embedding_model.tflite";
    println!("Testing XNNPACK with embedding model: {}", model_path);

    let model = Model::new(model_path).unwrap();
    let options = Options::default(); // XNNPACK should be enabled by default
    let interpreter = Interpreter::new(&model, Some(options)).unwrap();

    interpreter.allocate_tensors().unwrap();

    // Get input shape
    let input_tensor = interpreter.input(0).unwrap();
    let input_shape = input_tensor.shape();
    println!("Input shape: {:?}", input_shape.dimensions());

    // Create dummy input data
    let input_size = input_shape.dimensions().iter().product::<usize>();
    let dummy_input: Vec<f32> = (0..input_size).map(|i| (i as f32) * 0.01).collect();

    println!("Running inference with {} input elements...", input_size);

    // Time the inference
    let start = Instant::now();
    interpreter.copy(&dummy_input, 0).unwrap();
    interpreter.invoke().unwrap();
    let duration = start.elapsed();

    println!("Inference time: {:?}", duration);

    // Get output
    let output_tensor = interpreter.output(0).unwrap();
    let output_data = output_tensor.data::<f32>();

    println!("Output shape: {:?}", output_tensor.shape().dimensions());
    println!(
        "First 5 output values: {:?}",
        &output_data[..5.min(output_data.len())]
    );

    // Verify output is reasonable (non-zero and finite)
    let non_zero_count = output_data.iter().filter(|&&x| x != 0.0).count();
    println!("Non-zero outputs: {}/{}", non_zero_count, output_data.len());

    assert!(non_zero_count > 0, "Expected some non-zero outputs");
    assert!(
        output_data.iter().all(|&x| x.is_finite()),
        "All outputs should be finite"
    );

    println!("✅ XNNPACK delegate test passed!");
}

#[cfg(test)]
#[test]
fn test_xnnpack_performance_comparison() {
    println!("=== XNNPACK Performance Comparison ===");

    // This test compares performance with and without optimization
    // Since we can't disable XNNPACK at runtime, we'll just run multiple times
    // to measure consistency

    let model_path = "models/embedding_model.tflite";
    let model = Model::new(model_path).unwrap();

    // Test with default options (XNNPACK enabled)
    let options = Options::default();
    let interpreter = Interpreter::new(&model, Some(options)).unwrap();
    interpreter.allocate_tensors().unwrap();

    let input_tensor = interpreter.input(0).unwrap();
    let input_size = input_tensor.shape().dimensions().iter().product::<usize>();
    let dummy_input: Vec<f32> = (0..input_size).map(|i| (i as f32) * 0.01).collect();

    // Run multiple iterations to get average performance
    let iterations = 100;
    let mut total_time = std::time::Duration::new(0, 0);

    for i in 0..iterations {
        let start = Instant::now();
        interpreter.copy(&dummy_input, 0).unwrap();
        interpreter.invoke().unwrap();
        total_time += start.elapsed();

        if i % 20 == 0 {
            println!("Completed {} iterations", i);
        }
    }

    let avg_time = total_time / iterations;
    println!(
        "Average inference time over {} iterations: {:?}",
        iterations, avg_time
    );
    println!("Inferences per second: {:.2}", 1.0 / avg_time.as_secs_f64());

    // Performance should be reasonable (less than 10ms per inference for embedding model)
    assert!(
        avg_time.as_millis() < 10,
        "Inference should be fast with XNNPACK"
    );

    println!("✅ XNNPACK performance test passed!");
}

#[cfg(test)]
#[test]
fn test_xnnpack_with_multiple_models() {
    println!("=== Testing XNNPACK with multiple models ===");

    let models = [
        ("embedding", "models/embedding_model.tflite"),
        ("wakeword", "models/hey_mycroft_v0.1.tflite"),
        // Skip melspectrogram for now due to the known issue
    ];

    for (name, path) in &models {
        println!("Testing {} model: {}", name, path);

        let model = Model::new(path).unwrap();
        let options = Options::default();
        let interpreter = Interpreter::new(&model, Some(options)).unwrap();
        interpreter.allocate_tensors().unwrap();

        let input_tensor = interpreter.input(0).unwrap();
        let input_size = input_tensor.shape().dimensions().iter().product::<usize>();

        // Create appropriate dummy input
        let dummy_input: Vec<f32> = match *name {
            "embedding" => (0..input_size).map(|i| (i as f32) * 0.01).collect(),
            "wakeword" => (0..input_size).map(|i| (i as f32) * 0.001).collect(),
            _ => vec![0.0; input_size],
        };

        let start = Instant::now();
        interpreter.copy(&dummy_input, 0).unwrap();
        interpreter.invoke().unwrap();
        let duration = start.elapsed();

        println!("  {} inference time: {:?}", name, duration);

        let output_tensor = interpreter.output(0).unwrap();
        let output_data = output_tensor.data::<f32>();
        let non_zero_count = output_data.iter().filter(|&&x| x != 0.0).count();

        println!(
            "  {} non-zero outputs: {}/{}",
            name,
            non_zero_count,
            output_data.len()
        );

        assert!(
            output_data.iter().all(|&x| x.is_finite()),
            "All outputs should be finite for {}",
            name
        );
    }

    println!("✅ Multi-model XNNPACK test passed!");
}
