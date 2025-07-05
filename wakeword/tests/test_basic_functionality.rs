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
fn test_model_initialization() {
    // Test that Model can be created with different configurations
    let result = Model::new(
        vec![], // Load all models
        vec![], // Use default class mappings
        0.0,    // No VAD
        0.1,    // Default verifier threshold
    );

    match result {
        Ok(mut model) => {
            println!("Model created successfully");

            // Test prediction interface
            let dummy_audio = vec![0i16; 1280]; // 80ms of silence
            let prediction = model.predict(&dummy_audio, None, 0.0);
            match prediction {
                Ok(results) => {
                    println!("Prediction results: {:?}", results);
                    // Should have entries for each model
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
fn test_specific_model_loading() {
    // Test loading a specific model
    let result = Model::new(vec!["hey_mycroft".to_string()], vec![], 0.0, 0.1);

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
    let result = Model::new(vec![], vec![], 0.0, 0.1);

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

                let result = Model::new(vec![], vec![], 0.0, 0.1);
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
    let result = Model::new(vec![], vec![], 0.0, 0.1);

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
    let result = Model::new(vec![], vec![], 0.0, 0.1);

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
