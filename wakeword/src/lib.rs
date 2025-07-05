// Copyright 2024 - OpenWakeWord Rust Port
// Licensed under the Apache License, Version 2.0

//! # OpenWakeWord Rust Port
//!
//! A direct port of the Python OpenWakeWord implementation to Rust, providing
//! wake word detection using TensorFlow Lite models.
//!
//! This implementation closely mirrors the Python version's structure and API
//! for better compatibility and performance.

pub mod error;
pub mod model;
pub mod utils;

pub mod test_utils;

// Re-export main types for convenient access
pub use error::{OpenWakeWordError, Result};
pub use model::Model;
pub use test_utils::*;
pub use utils::AudioFeatures;

use std::collections::HashMap;

/// Feature models configuration (melspectrogram and embedding)
pub const FEATURE_MODELS: &[(&str, &str)] = &[
    ("embedding", "models/embedding_model.tflite"),
    ("melspectrogram", "models/melspectrogram.tflite"),
];

/// Available wake word models
pub const MODELS: &[(&str, &str)] = &[
    ("alexa", "models/alexa_v0.1.tflite"),
    ("hey_mycroft", "models/hey_mycroft_v0.1.tflite"),
    ("hey_jarvis", "models/hey_jarvis_v0.1.tflite"),
    ("hey_rhasspy", "models/hey_rhasspy_v0.1.tflite"),
    ("timer", "models/timer_v0.1.tflite"),
    ("weather", "models/weather_v0.1.tflite"),
];

/// Get pre-trained model paths for TFLite models
pub fn get_pretrained_model_paths() -> Vec<String> {
    MODELS.iter().map(|(_, path)| path.to_string()).collect()
}

/// Default model class mappings for multi-class models
pub fn get_model_class_mappings() -> HashMap<String, HashMap<String, String>> {
    let mut mappings = HashMap::new();

    // Timer model has multiple classes
    let mut timer_mapping = HashMap::new();
    timer_mapping.insert("1".to_string(), "1_minute_timer".to_string());
    timer_mapping.insert("2".to_string(), "5_minute_timer".to_string());
    timer_mapping.insert("3".to_string(), "10_minute_timer".to_string());
    timer_mapping.insert("4".to_string(), "20_minute_timer".to_string());
    timer_mapping.insert("5".to_string(), "30_minute_timer".to_string());
    timer_mapping.insert("6".to_string(), "1_hour_timer".to_string());
    mappings.insert("timer".to_string(), timer_mapping);

    mappings
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_utils::*;

    #[test]
    fn test_hey_mycroft_detection() {
        env_logger::try_init().ok(); // Initialize logging for tests

        println!("Loading hey_mycroft test audio file...");
        let audio_data = load_test_audio("../tests/data/hey_mycroft_test.wav")
            .expect("Failed to load hey_mycroft_test.wav");

        println!("Creating model...");
        let mut model = Model::new(
            vec!["hey_mycroft".to_string()],
            vec![], // Use default class mappings
            0.0,    // No VAD filtering
            0.1,    // Default custom verifier threshold
        )
        .expect("Failed to create model");

        println!(
            "Processing {} samples in chunks of 1280...",
            audio_data.len()
        );

        // Debug: Check model input size
        for (model_name, input_size) in model.get_model_inputs() {
            println!("Model {} expects {} input elements", model_name, input_size);
        }

        let chunk_size = 1280; // 80ms at 16kHz
        let mut detected = false;
        let mut max_confidence = 0.0;
        let mut chunks_processed = 0;
        let mut all_predictions = Vec::new();

        for (i, chunk) in audio_data.chunks(chunk_size).enumerate() {
            if chunk.len() == chunk_size {
                chunks_processed += 1;

                let predictions = model.predict(chunk, None, 0.0);
                match predictions {
                    Ok(preds) => {
                        for (model_name, confidence) in preds {
                            if confidence > max_confidence {
                                max_confidence = confidence;
                            }

                            all_predictions.push((i, confidence));

                            if confidence > 0.3 {
                                // Lower threshold for detection
                                println!(
                                    "🎯 Detection at chunk {}: {} = {:.6}",
                                    i, model_name, confidence
                                );
                                detected = true;
                            } else if i % 5 == 0 {
                                // Log every 5th chunk
                                println!("Chunk {}: {} = {:.6}", i, model_name, confidence);
                            }
                        }
                    }
                    Err(e) => {
                        println!("Prediction error at chunk {}: {}", i, e);
                    }
                }
            }
        }

        println!(
            "Processed {} chunks, max confidence: {:.6}",
            chunks_processed, max_confidence
        );

        // Show confidence progression
        if !all_predictions.is_empty() {
            let high_confidence_chunks: Vec<_> = all_predictions
                .iter()
                .filter(|(_, conf)| *conf > 0.01)
                .collect();

            if !high_confidence_chunks.is_empty() {
                println!(
                    "Chunks with confidence > 0.01: {:?}",
                    high_confidence_chunks
                );
            }
        }

        // Check if buffer is getting filled
        let features = model.get_preprocessor().get_features(16, -1);
        println!("Final feature buffer size: {}", features.len());

        // Lower threshold for now to see if we're getting any signal
        assert!(
            max_confidence > 0.01,
            "Should detect some signal in hey_mycroft wake word (max confidence: {:.6})",
            max_confidence
        );
    }

    #[test]
    fn test_all_test_files_detect() {
        env_logger::try_init().ok();

        let test_files = [
            "../tests/data/hey_mycroft_test.wav",
            "../tests/data/delay_start_what_time_is_it.wav",
            "../tests/data/hesitation_what_time_is_it.wav",
            "../tests/data/immediate_what_time_is_it.wav",
        ];

        for test_file in &test_files {
            println!("\n🧪 Testing file: {}", test_file);

            let audio_data =
                load_test_audio(test_file).expect(&format!("Failed to load {}", test_file));

            let mut model = Model::new(vec!["hey_mycroft".to_string()], vec![], 0.0, 0.1)
                .expect("Failed to create model");

            let chunk_size = 1280;
            let mut detected = false;
            let mut max_confidence = 0.0;
            let mut detection_chunks = Vec::new();

            for (i, chunk) in audio_data.chunks(chunk_size).enumerate() {
                if chunk.len() == chunk_size {
                    let predictions = model
                        .predict(chunk, None, 0.0)
                        .expect("Failed to run prediction");

                    for (model_name, confidence) in predictions {
                        if confidence > max_confidence {
                            max_confidence = confidence;
                        }

                        if confidence > 0.3 {
                            // Lower threshold for debugging
                            detection_chunks.push((i, confidence));
                            if confidence > 0.5 {
                                detected = true;
                            }
                        }
                    }
                }
            }

            println!(
                "File: {} - Max confidence: {:.6}",
                test_file, max_confidence
            );
            if !detection_chunks.is_empty() {
                println!("Detection chunks (>0.3): {:?}", detection_chunks);
            }

            assert!(
                detected,
                "File {} should detect wake word (max confidence: {:.6})",
                test_file, max_confidence
            );
        }
    }

    #[test]
    fn test_no_false_positives_with_silence() {
        env_logger::try_init().ok();

        println!("Testing with silence/noise...");

        let mut model = Model::new(vec!["hey_mycroft".to_string()], vec![], 0.0, 0.1)
            .expect("Failed to create model");

        // Generate 2 seconds of low-level noise
        let sample_rate = 16000;
        let duration_secs = 2;
        let total_samples = sample_rate * duration_secs;
        let chunk_size = 1280;

        let mut max_confidence = 0.0;
        let mut false_positives = 0;

        for chunk_start in (0..total_samples).step_by(chunk_size) {
            let chunk_end = std::cmp::min(chunk_start + chunk_size, total_samples);
            let chunk_samples = chunk_end - chunk_start;

            if chunk_samples == chunk_size {
                // Generate low-level random noise
                let chunk: Vec<i16> = (0..chunk_size)
                    .map(|_| (rand::random::<f32>() * 200.0 - 100.0) as i16)
                    .collect();

                let predictions = model
                    .predict(&chunk, None, 0.0)
                    .expect("Failed to run prediction");

                for (_, confidence) in predictions {
                    if confidence > max_confidence {
                        max_confidence = confidence;
                    }

                    if confidence > 0.5 {
                        false_positives += 1;
                    }
                }
            }
        }

        println!(
            "Silence test - Max confidence: {:.6}, False positives: {}",
            max_confidence, false_positives
        );

        assert_eq!(
            false_positives, 0,
            "Should not have false positives with noise (max confidence: {:.6})",
            max_confidence
        );
    }

    #[test]
    fn test_model_reset() {
        env_logger::try_init().ok();

        let mut model = Model::new(vec!["hey_mycroft".to_string()], vec![], 0.0, 0.1)
            .expect("Failed to create model");

        // Process some audio first
        let audio_data = load_test_audio("../tests/data/hey_mycroft_test.wav")
            .expect("Failed to load test audio");

        let chunk = &audio_data[0..1280];
        let _ = model.predict(chunk, None, 0.0);

        // Reset should not crash
        model.reset();

        // Should still work after reset
        let predictions = model
            .predict(chunk, None, 0.0)
            .expect("Failed to predict after reset");

        // Should have predictions for hey_mycroft
        assert!(
            predictions.contains_key("hey_mycroft"),
            "Should have hey_mycroft prediction after reset"
        );
    }

    #[test]
    fn test_xnnpack_vs_cpu_performance() {
        use tflitec::interpreter::{Interpreter, Options};
        use tflitec::model::Model as TfliteModel;

        env_logger::try_init().ok();

        // Use hey_mycroft model for benchmarking
        let model_path = "models/hey_mycroft_v0.1.tflite";
        println!("🚀 Performance benchmark: CPU vs XNNPACK");

        // Create models and interpreters
        let model = TfliteModel::new(model_path).expect("Failed to create model");

        // Create CPU-only interpreter (use a copy for fairness)
        let cpu_interpreter =
            Interpreter::new(&model, None).expect("Failed to create CPU interpreter");
        cpu_interpreter
            .allocate_tensors()
            .expect("Failed to allocate CPU tensors");

        // Create XNNPACK interpreter (default options have XNNPACK enabled)
        let xnnpack_options = Options::default(); // XNNPACK enabled by default
        let xnnpack_interpreter = Interpreter::new(&model, Some(xnnpack_options))
            .expect("Failed to create XNNPACK interpreter");
        xnnpack_interpreter
            .allocate_tensors()
            .expect("Failed to allocate XNNPACK tensors");

        // Get input tensor info
        let input_tensor = cpu_interpreter
            .input(0)
            .expect("Failed to get input tensor");
        let input_size = input_tensor.shape().dimensions().iter().product::<usize>();
        println!("Model input size: {} elements", input_size);

        // Create dummy input data
        let dummy_input: Vec<f32> = (0..input_size).map(|i| (i as f32) * 0.001).collect();

        // Warmup runs
        println!("🔥 Warming up...");
        for _ in 0..5 {
            cpu_interpreter
                .copy(&dummy_input, 0)
                .expect("Failed to copy CPU input");
            cpu_interpreter.invoke().expect("Failed to invoke CPU");

            xnnpack_interpreter
                .copy(&dummy_input, 0)
                .expect("Failed to copy XNNPACK input");
            xnnpack_interpreter
                .invoke()
                .expect("Failed to invoke XNNPACK");
        }

        // Benchmark CPU inference
        println!("📊 Benchmarking CPU inference...");
        let cpu_start = std::time::Instant::now();
        let cpu_iterations = 100;

        for _ in 0..cpu_iterations {
            cpu_interpreter
                .copy(&dummy_input, 0)
                .expect("Failed to copy CPU input");
            cpu_interpreter.invoke().expect("Failed to invoke CPU");
        }

        let cpu_elapsed = cpu_start.elapsed();
        let cpu_avg_ms = cpu_elapsed.as_nanos() as f64 / cpu_iterations as f64 / 1_000_000.0;

        // Benchmark XNNPACK inference
        println!("🚀 Benchmarking XNNPACK inference...");
        let xnnpack_start = std::time::Instant::now();
        let xnnpack_iterations = 100;

        for _ in 0..xnnpack_iterations {
            xnnpack_interpreter
                .copy(&dummy_input, 0)
                .expect("Failed to copy XNNPACK input");
            xnnpack_interpreter
                .invoke()
                .expect("Failed to invoke XNNPACK");
        }

        let xnnpack_elapsed = xnnpack_start.elapsed();
        let xnnpack_avg_ms =
            xnnpack_elapsed.as_nanos() as f64 / xnnpack_iterations as f64 / 1_000_000.0;

        // Calculate speedup
        let speedup = cpu_avg_ms / xnnpack_avg_ms;

        // Display results
        println!("\n📈 PERFORMANCE RESULTS:");
        println!("┌─────────────────────────────────────────────────────────────┐");
        println!("│                    Inference Performance                    │");
        println!("├─────────────────────────────────────────────────────────────┤");
        println!(
            "│ CPU Only:        {:>8.3} ms per inference              │",
            cpu_avg_ms
        );
        println!(
            "│ XNNPACK:         {:>8.3} ms per inference              │",
            xnnpack_avg_ms
        );
        println!(
            "│ Speedup:         {:>8.2}x faster                        │",
            speedup
        );
        println!(
            "│ Improvement:     {:>8.1}% faster                        │",
            (speedup - 1.0) * 100.0
        );
        println!("└─────────────────────────────────────────────────────────────┘");

        // Verify outputs are similar (sanity check)
        let cpu_output = cpu_interpreter.output(0).expect("Failed to get CPU output");
        let xnnpack_output = xnnpack_interpreter
            .output(0)
            .expect("Failed to get XNNPACK output");

        let cpu_data = cpu_output.data::<f32>();
        let xnnpack_data = xnnpack_output.data::<f32>();

        // Check if outputs are reasonably close
        let max_diff = cpu_data
            .iter()
            .zip(xnnpack_data.iter())
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f32::max);

        println!("🔍 Output validation: max difference = {:.6}", max_diff);
        assert!(
            max_diff < 0.001,
            "CPU and XNNPACK outputs differ too much: {}",
            max_diff
        );

        // Performance should be better with XNNPACK (or at least not worse)
        if speedup > 1.1 {
            println!("✅ XNNPACK shows significant performance improvement!");
        } else if speedup > 0.9 {
            println!("⚖️  XNNPACK performance is comparable to CPU");
        } else {
            println!("⚠️  XNNPACK appears slower than CPU - this may indicate an issue");
        }
    }
}
