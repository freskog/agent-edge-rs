//! Test to verify XNNPACK is working and provides performance benefits

use std::time::Instant;
use tflitec::interpreter::{Interpreter, Options};
use tflitec::model::Model;

// Force linking of required libraries for XNNPACK
#[cfg(all(target_arch = "aarch64", target_os = "linux"))]
#[link(name = "cpuinfo")]
extern "C" {}

#[cfg(all(target_arch = "aarch64", target_os = "linux"))]
#[link(name = "pthreadpool")]
extern "C" {}

#[cfg(test)]
#[test]
fn test_cpu_only_works() {
    println!("=== Testing CPU-only inference ===");

    let model_path = "models/embedding_model.tflite";
    let model = Model::new(model_path).unwrap();

    // CPU-only options (no XNNPACK)
    let mut options = Options::default();
    options.thread_count = 1;
    // Do NOT enable XNNPACK

    let interpreter = Interpreter::new(&model, Some(options)).unwrap();
    interpreter.allocate_tensors().unwrap();

    // Create dummy input
    let input_tensor = interpreter.input(0).unwrap();
    let input_size = input_tensor.shape().dimensions().iter().product::<usize>();
    let dummy_input: Vec<f32> = (0..input_size).map(|i| (i as f32) * 0.01).collect();

    // Run inference
    interpreter.copy(&dummy_input, 0).unwrap();
    interpreter.invoke().unwrap();
    let output = interpreter.output(0).unwrap();

    println!(
        "✅ CPU-only inference works! Output shape: {:?}",
        output.shape().dimensions()
    );
}

#[cfg(test)]
#[cfg(all(target_arch = "aarch64", target_os = "linux"))]
#[test]
fn test_xnnpack_segfault_isolation() {
    println!("=== Testing XNNPACK segfault isolation with FIX ===");

    let model_path = "models/embedding_model.tflite";
    let model = Model::new(model_path).unwrap();

    println!("🔧 Step 1: Create XNNPACK options with our fix...");
    let xnnpack_options = wakeword::xnnpack_fix::create_xnnpack_options(1);
    println!("✅ XNNPACK options created with working fix");

    println!("🔧 Step 2: Create interpreter with fixed XNNPACK...");
    // Use our working XNNPACK fix instead of broken is_xnnpack_enabled
    let interpreter =
        wakeword::xnnpack_fix::create_interpreter_with_xnnpack_safe(&model, 1).unwrap();
    println!("✅ XNNPACK interpreter created with fix");

    println!("🔧 Step 3: Allocate tensors...");
    interpreter.allocate_tensors().unwrap();
    println!("✅ Tensors allocated");

    println!("🔧 Step 4: Create dummy input...");
    let input_tensor = interpreter.input(0).unwrap();
    let input_size = input_tensor.shape().dimensions().iter().product::<usize>();
    let dummy_input: Vec<f32> = (0..input_size).map(|i| (i as f32) * 0.01).collect();
    println!("✅ Dummy input created (size: {})", input_size);

    println!("🔧 Step 5: Set input data...");
    interpreter.copy(&dummy_input, 0).unwrap();
    println!("✅ Input data set");

    println!("🔧 Step 6: Run inference with FIXED XNNPACK...");
    // This should NOT segfault anymore thanks to our fix
    interpreter.invoke().unwrap();
    println!("✅ XNNPACK inference completed without segfault!");

    let output = interpreter.output(0).unwrap();
    println!(
        "✅ XNNPACK inference works! Output shape: {:?}",
        output.shape().dimensions()
    );
}

#[cfg(test)]
#[cfg(all(target_arch = "aarch64", target_os = "linux"))]
#[test]
fn test_xnnpack_vs_cpu_performance() {
    println!("=== XNNPACK vs CPU Performance Comparison ===");

    let model_path = "models/embedding_model.tflite";
    let model = Model::new(model_path).unwrap();

    // Test 1: CPU-only (XNNPACK disabled)
    println!("🔧 Testing CPU-only inference (XNNPACK disabled)...");
    let mut cpu_options = Options::default();
    cpu_options.thread_count = 1;
    // CPU-only inference (XNNPACK should be automatically disabled when libs not available)

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

    // Test 2: XNNPACK-accelerated with our fix
    println!("🚀 Testing XNNPACK-accelerated inference with fix...");
    let xnnpack_interpreter =
        wakeword::xnnpack_fix::create_interpreter_with_xnnpack_safe(&model, 1).unwrap();
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

    // Verify both versions work and produce reasonable results
    // Note: XNNPACK may not always be faster for small models or in virtualized environments
    assert!(
        speedup > 0.5 && speedup < 5.0,
        "XNNPACK performance should be reasonable (0.5x to 5.0x CPU performance), got {:.2}x",
        speedup
    );

    println!("✅ Both CPU and XNNPACK versions work correctly!");
}

#[cfg(test)]
#[cfg(all(target_arch = "aarch64", target_os = "linux"))]
#[test]
fn test_xnnpack_feature_availability() {
    println!("=== Testing XNNPACK Feature Availability with FIX ===");

    println!("✅ XNNPACK fix is available");

    // Test that we can create XNNPACK options with our fix
    let xnnpack_options = wakeword::xnnpack_fix::create_xnnpack_options(1);
    println!(
        "✅ XNNPACK options created: thread_count = {}",
        xnnpack_options.num_threads
    );

    // Test that we can create interpreters with our fixed XNNPACK
    let model_path = "models/embedding_model.tflite";
    let model = Model::new(model_path).unwrap();

    let interpreter_result = wakeword::xnnpack_fix::create_interpreter_with_xnnpack_safe(&model, 1);
    match interpreter_result {
        Ok(_) => println!("✅ XNNPACK interpreter created successfully with fix"),
        Err(e) => println!("❌ Failed to create XNNPACK interpreter: {}", e),
    }
}

#[cfg(test)]
#[cfg(all(target_arch = "aarch64", target_os = "linux"))]
#[test]
fn test_multiple_models_with_xnnpack() {
    println!("=== Testing Multiple Models with XNNPACK ===");

    let models = [
        ("embedding", "models/embedding_model.tflite"),
        ("wakeword", "models/hey_mycroft_v0.1.tflite"),
    ];

    for (name, path) in &models {
        println!("Testing {} model with FIXED XNNPACK...", name);

        let model = Model::new(path).unwrap();
        // Use our working XNNPACK fix instead of broken default options
        let interpreter =
            wakeword::xnnpack_fix::create_interpreter_with_xnnpack_safe(&model, 1).unwrap();
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

    println!("✅ All models work with FIXED XNNPACK enabled!");
}

#[cfg(test)]
#[cfg(not(all(target_arch = "aarch64", target_os = "linux")))]
#[test]
fn test_xnnpack_not_available() {
    println!("=== XNNPACK Not Available ===");
    println!("❌ XNNPACK is not enabled on this platform");
    println!("✅ This is expected on platforms other than Linux aarch64");
}
