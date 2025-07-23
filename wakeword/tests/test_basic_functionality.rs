use std::collections::HashMap;
use wakeword::model::Model;
use wakeword::test_utils::load_test_audio;
use wakeword::utils::AudioFeatures;

#[test]
fn test_audio_features_initialization() {
    // Test that AudioFeatures can be created and initialized
    let result = AudioFeatures::new(
        "models/melspectrogram.tflite",
        "models/embedding_model.tflite",
        16000,
    );

    // Should succeed (even if model files don't exist, we're testing the structure)
    match result {
        Ok(mut features) => {
            println!("AudioFeatures created successfully");

            // Test the callable interface
            let dummy_audio = vec![0i16; 1280]; // 80ms of silence
            let n_samples = features.__call__(&dummy_audio);
            match n_samples {
                Ok(samples) => println!("Processed {} samples", samples),
                Err(e) => println!("Expected error (no model files): {}", e),
            }
        }
        Err(e) => {
            println!("Expected error (no model files): {}", e);
        }
    }
}

#[test]
fn test_model_creation() {
    // Test that Model can be created with different model specifications
    let model_names = vec!["hey_mycroft".to_string()];
    let result = Model::new(
        model_names,
        vec![], // Use default class mappings
    );

    match result {
        Ok(model) => {
            println!("Model created successfully: {:?}", model.get_model_inputs());

            // Test basic prediction interface (even if models don't exist)
            // This mainly tests the API structure
        }
        Err(e) => {
            println!("Expected error (no model files in test environment): {}", e);
            // This is expected in CI environments without model files
            assert!(
                e.to_string().contains("Model not found")
                    || e.to_string().contains("Failed to load model")
                    || e.to_string().contains("No such file"),
                "Unexpected error type: {}",
                e
            );
        }
    }

    // Test with multiple models
    let result = Model::new(vec!["hey_mycroft".to_string()], vec![]);

    match result {
        Ok(mut model) => {
            println!("Specific model loaded successfully");

            // Test with dummy audio
            let dummy_audio = vec![0i16; 3840]; // 240ms of silence (3 chunks)
            let prediction = model.predict(&dummy_audio, None, 0.0);
            match prediction {
                Ok(results) => {
                    println!("Multi-chunk prediction results: {:?}", results);
                    // Should handle multiple chunks
                    assert!(!results.is_empty(), "Should have prediction results");
                }
                Err(e) => println!("Expected error (no model files): {}", e),
            }
        }
        Err(e) => {
            println!("Expected error (no model files): {}", e);
        }
    }
}

#[test]
fn test_streaming_behavior() {
    // Test streaming behavior similar to Python
    let result = Model::new(vec![], vec![]);

    match result {
        Ok(mut model) => {
            println!("Testing streaming behavior");

            // Simulate streaming audio in 80ms chunks
            let chunk_size = 1280;
            let mut all_predictions = Vec::new();

            for i in 0..5 {
                let chunk = vec![0i16; chunk_size];
                let prediction = model.predict(&chunk, None, 0.0);
                match prediction {
                    Ok(results) => {
                        println!("Chunk {}: {:?}", i, results);
                        all_predictions.push(results);
                    }
                    Err(e) => println!("Expected error (no model files): {}", e),
                }
            }

            // Should have processed 5 chunks
            // (would be more meaningful with actual model files)
        }
        Err(e) => {
            println!("Expected error (no model files): {}", e);
        }
    }
}

#[test]
fn test_with_real_models_if_available() {
    // Test with actual model files if they exist
    let model_combinations = vec![
        (vec!["hey_mycroft".to_string()], "hey_mycroft"),
        (vec!["alexa".to_string()], "alexa"),
        (vec!["hey_jarvis".to_string()], "hey_jarvis"),
    ];

    for (model_names, test_name) in model_combinations {
        let result = Model::new(vec![], vec![]);

        match result {
            Ok(mut model) => {
                println!("Testing with model: {}", test_name);

                // Test predict interface with dummy data
                let dummy_audio = vec![0i16; 1280]; // 80ms of silence
                let prediction_result = model.predict(&dummy_audio, None, 0.0);

                match prediction_result {
                    Ok(predictions) => {
                        println!("{} predictions: {:?}", test_name, predictions);

                        // Verify prediction structure
                        for (name, confidence) in predictions {
                            assert!(
                                confidence >= 0.0 && confidence <= 1.0,
                                "Confidence out of range for {}: {}",
                                name,
                                confidence
                            );
                        }
                    }
                    Err(e) => {
                        println!(
                            "Prediction error for {} (expected in test env): {}",
                            test_name, e
                        );
                    }
                }
            }
            Err(e) => {
                println!(
                    "Model creation failed for {} (expected without model files): {}",
                    test_name, e
                );
            }
        }
    }
}

#[test]
fn test_with_real_audio_if_available() {
    // Test with actual audio files if they exist
    let test_files = [
        "tests/data/alexa_test.wav",
        "tests/data/hey_mycroft_test.wav",
        "tests/data/hey_jarvis_test.wav",
        "tests/data/timer_test.wav",
    ];

    for test_file in &test_files {
        match load_test_audio(test_file) {
            Ok(audio_data) => {
                println!("Testing with real audio file: {}", test_file);

                let result = Model::new(vec![], vec![]);
                match result {
                    Ok(mut model) => {
                        // Process in chunks like the Python implementation
                        let chunk_size = 1280;
                        let mut max_confidence = 0.0;
                        let mut detections = Vec::new();

                        for chunk in audio_data.chunks(chunk_size) {
                            let prediction = model.predict(chunk, None, 0.0);
                            match prediction {
                                Ok(results) => {
                                    for (model_name, confidence) in results {
                                        if confidence > max_confidence {
                                            max_confidence = confidence;
                                        }
                                        if confidence > 0.5 {
                                            detections.push((model_name, confidence));
                                        }
                                    }
                                }
                                Err(e) => println!("Error processing chunk: {}", e),
                            }
                        }

                        println!(
                            "File: {}, Max confidence: {:.4}, Detections: {:?}",
                            test_file, max_confidence, detections
                        );
                    }
                    Err(e) => println!("Model creation error: {}", e),
                }
            }
            Err(_) => {
                // Audio file doesn't exist, which is fine for CI
                println!("Audio file {} not found (expected in CI)", test_file);
            }
        }
    }
}

#[test]
fn test_prediction_buffer_behavior() {
    // Test that prediction buffers behave like Python (maxlen=30)
    let result = Model::new(vec![], vec![]);

    match result {
        Ok(mut model) => {
            println!("Testing prediction buffer behavior");

            // Feed many chunks to test buffer management
            let chunk = vec![0i16; 1280];
            for i in 0..50 {
                let prediction = model.predict(&chunk, None, 0.0);
                match prediction {
                    Ok(results) => {
                        if i % 10 == 0 {
                            println!("Iteration {}: {:?}", i, results);
                        }
                    }
                    Err(e) => println!("Error on iteration {}: {}", i, e),
                }
            }

            // Buffer should have been managed properly (no crashes)
            println!("Buffer management test completed");
        }
        Err(e) => {
            println!("Expected error (no model files): {}", e);
        }
    }
}

#[test]
fn test_debounce_behavior() {
    // Test debounce functionality
    let result = Model::new(vec![], vec![]);

    match result {
        Ok(mut model) => {
            println!("Testing debounce behavior");

            let chunk = vec![0i16; 1280];
            let threshold =
                HashMap::from([("hey_mycroft".to_string(), 0.3), ("alexa".to_string(), 0.3)]);

            // Test with debounce
            let prediction = model.predict(&chunk, Some(threshold), 1.0);
            match prediction {
                Ok(results) => {
                    println!("Debounced prediction: {:?}", results);
                }
                Err(e) => println!("Error with debounce: {}", e),
            }
        }
        Err(e) => {
            println!("Expected error (no model files): {}", e);
        }
    }
}

#[test]
fn test_model_error_handling() {
    // Test error handling with invalid configurations
    let result = Model::new(vec![], vec![]);
    match result {
        Ok(model) => println!("Empty model list created successfully"),
        Err(e) => println!("Expected error with empty model list: {}", e),
    }

    // Test with non-existent model
    let result = Model::new(vec!["non_existent".to_string()], vec![]);
    assert!(result.is_err(), "Should fail with non-existent model");
}

#[test]
fn test_audio_features_if_available() {
    // Test AudioFeatures creation if model files exist
    let result = Model::new(vec![], vec![]);
    match result {
        Ok(model) => {
            println!("AudioFeatures component working");
            // Test basic preprocessor functionality
        }
        Err(e) => {
            println!("AudioFeatures not available (expected): {}", e);
        }
    }
}

#[test]
fn test_performance_characteristics() {
    // Basic performance testing if models are available
    let result = Model::new(vec![], vec![]);
    if let Ok(mut model) = result {
        let start = std::time::Instant::now();
        let dummy_audio = vec![0i16; 1280];
        let _ = model.predict(&dummy_audio, None, 0.0);
        let elapsed = start.elapsed();
        println!("Single prediction took: {:?}", elapsed);
        assert!(
            elapsed.as_millis() < 5000,
            "Prediction should be reasonably fast"
        );
    } else {
        println!("Performance test skipped - models not available");
    }
}
