//! Test to verify XNNPACK is working and provides performance benefits

use std::time::Instant;
use tflitec::interpreter::{Interpreter, Options};
use tflitec::model::Model;

// Force linking of required libraries for XNNPACK
#[cfg(target_arch = "aarch64")]
#[link(name = "cpuinfo")]
extern "C" {}

#[cfg(target_arch = "aarch64")]
#[link(name = "pthreadpool")]
extern "C" {}

#[cfg(test)]
#[test]
fn test_xnnpack_vs_cpu_performance() {
    println!("=== XNNPACK vs CPU Performance Comparison ===");

    let model_path = "models/embedding_model.tflite";
    let model = Model::new(model_path).unwrap();

    // Test 1: CPU-only (XNNPACK disabled)
    println!("🔧 Testing CPU-only inference (XNNPACK disabled)...");
    let mut cpu_options = Options::default();
    cpu_options.thread_count = 1;
    #[cfg(feature = "xnnpack")]
    {
        cpu_options.is_xnnpack_enabled = false; // Explicitly disable XNNPACK
    }

    let cpu_interpreter = Interpreter::new(&model, Some(cpu_options)).unwrap();
    cpu_interpreter.allocate_tensors().unwrap();

    // Get input/output info for CPU test
    let input_tensor = cpu_interpreter.input(0).unwrap();
    let input_size = input_tensor.shape().dimensions().iter().product::<usize>();
    let dummy_input: Vec<f32> = (0..input_size).map(|i| (i as f32) * 0.01).collect();

    // Warm up and benchmark CPU
    let mut cpu_times = Vec::new();
    for _ in 0..10 {
        let start = Instant::now();

        cpu_interpreter.copy(&dummy_input, 0).unwrap();
        cpu_interpreter.invoke().unwrap();
        let _output = cpu_interpreter.output(0).unwrap();

        cpu_times.push(start.elapsed());
    }

    let cpu_avg = cpu_times.iter().sum::<std::time::Duration>() / cpu_times.len() as u32;
    let cpu_inferences_per_sec = 1.0 / cpu_avg.as_secs_f64();

    println!("  CPU-only average time: {:?}", cpu_avg);
    println!("  CPU-only inferences/sec: {:.2}", cpu_inferences_per_sec);

    // Test 2: XNNPACK-accelerated
    println!("🚀 Testing XNNPACK-accelerated inference...");
    let mut xnnpack_options = Options::default();
    xnnpack_options.thread_count = 1;
    #[cfg(feature = "xnnpack")]
    {
        xnnpack_options.is_xnnpack_enabled = true; // Enable XNNPACK
    }

    let xnnpack_interpreter = Interpreter::new(&model, Some(xnnpack_options)).unwrap();
    xnnpack_interpreter.allocate_tensors().unwrap();

    // Get input/output info for XNNPACK test
    let _input_tensor = xnnpack_interpreter.input(0).unwrap();

    // Warm up and benchmark XNNPACK
    let mut xnnpack_times = Vec::new();
    for _ in 0..10 {
        let start = Instant::now();

        xnnpack_interpreter.copy(&dummy_input, 0).unwrap();
        xnnpack_interpreter.invoke().unwrap();
        let _output = xnnpack_interpreter.output(0).unwrap();

        xnnpack_times.push(start.elapsed());
    }

    let xnnpack_avg =
        xnnpack_times.iter().sum::<std::time::Duration>() / xnnpack_times.len() as u32;
    let xnnpack_inferences_per_sec = 1.0 / xnnpack_avg.as_secs_f64();

    println!("  XNNPACK average time: {:?}", xnnpack_avg);
    println!(
        "  XNNPACK inferences/sec: {:.2}",
        xnnpack_inferences_per_sec
    );

    // Compare performance
    let speedup = cpu_avg.as_secs_f64() / xnnpack_avg.as_secs_f64();
    println!("🎯 XNNPACK speedup: {:.2}x faster than CPU", speedup);

    // Verify XNNPACK is actually faster (it should be!)
    assert!(
        speedup > 1.0,
        "XNNPACK should be faster than CPU-only inference"
    );
}

#[cfg(test)]
#[test]
fn test_xnnpack_feature_availability() {
    println!("=== Testing XNNPACK Feature Availability ===");

    #[cfg(feature = "xnnpack")]
    {
        println!("✅ XNNPACK feature is enabled");

        // Test that we can create interpreters with XNNPACK options
        let model_path = "models/embedding_model.tflite";
        let model = Model::new(model_path).unwrap();

        let mut options = Options::default();
        options.is_xnnpack_enabled = true;

        let interpreter = Interpreter::new(&model, Some(options));
        match interpreter {
            Ok(_) => println!("✅ XNNPACK interpreter created successfully"),
            Err(e) => println!("❌ Failed to create XNNPACK interpreter: {}", e),
        }
    }

    #[cfg(not(feature = "xnnpack"))]
    {
        println!("❌ XNNPACK feature is NOT enabled");
        panic!("XNNPACK feature should be enabled for this test");
    }
}

#[cfg(test)]
#[test]
fn test_multiple_models_with_xnnpack() {
    println!("=== Testing Multiple Models with XNNPACK ===");

    let models = [
        ("embedding", "models/embedding_model.tflite"),
        ("wakeword", "models/hey_mycroft_v0.1.tflite"),
    ];

    for (name, path) in &models {
        println!("Testing {} model with XNNPACK...", name);

        let model = Model::new(path).unwrap();
        let mut options = Options::default();
        options.thread_count = 1;
        #[cfg(feature = "xnnpack")]
        {
            options.is_xnnpack_enabled = true;
        }

        let interpreter = Interpreter::new(&model, Some(options)).unwrap();
        interpreter.allocate_tensors().unwrap();

        let input_tensor = interpreter.input(0).unwrap();
        let input_size = input_tensor.shape().dimensions().iter().product::<usize>();

        let dummy_input: Vec<f32> = match *name {
            "embedding" => (0..input_size).map(|i| (i as f32) * 0.01).collect(),
            "wakeword" => (0..input_size).map(|i| (i as f32) * 0.001).collect(),
            _ => vec![0.0; input_size],
        };

        // Time inference
        let start = Instant::now();
        interpreter.copy(&dummy_input, 0).unwrap();
        interpreter.invoke().unwrap();
        let duration = start.elapsed();

        println!("  {} XNNPACK inference time: {:?}", name, duration);

        // Verify output
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
            "All outputs should be finite"
        );
        assert!(non_zero_count > 0, "Should have some non-zero outputs");
    }

    println!("✅ All models work with XNNPACK enabled!");
}
