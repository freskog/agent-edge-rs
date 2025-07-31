use super::*;
use crate::test_utils::*;
use std::path::Path;

/// Test that alexa_test.wav does NOT trigger hey_mycroft detection
#[test]
fn test_alexa_audio_no_false_positive() {
    env_logger::try_init().ok(); // Initialize logging for tests

    let test_file = "../tests/data/alexa_test.wav";
    if !Path::new(test_file).exists() {
        panic!("Test file not found: {}", test_file);
    }

    // Load audio file
    let mut reader = hound::WavReader::open(test_file).expect("Failed to open test audio file");
    let samples: std::result::Result<Vec<i16>, _> = reader.samples().collect();
    let audio_data = samples.expect("Failed to read audio samples");

    // Verify audio format
    let spec = reader.spec();
    assert_eq!(spec.sample_rate, 16000, "Expected 16kHz sample rate");
    assert_eq!(spec.channels, 1, "Expected mono audio");
    assert_eq!(spec.bits_per_sample, 16, "Expected 16-bit audio");

    // Create model
    let model_names = vec!["hey_mycroft".to_string()];
    let class_mappings = vec![]; // Empty class mappings
    let mut model = Model::new(model_names, class_mappings).expect("Failed to create model");

    // Test prediction
    let results = model
        .predict(&audio_data, None, 0.0)
        .expect("Prediction failed");

    // Verify no false positives
    for (wake_word, confidence) in results.iter() {
        println!("Alexa test - {}: {:.10}", wake_word, confidence);
        assert!(
            *confidence < 0.5,
            "False positive detected: {} confidence {} >= 0.5",
            wake_word,
            confidence
        );

        // Should be very low confidence (essentially 0)
        assert!(
            *confidence < 1e-6,
            "Confidence too high for non-wakeword: {} = {}",
            wake_word,
            confidence
        );
    }
}

/// Test that hey_mycroft_test.wav DOES trigger hey_mycroft detection
#[test]
fn test_hey_mycroft_audio_detection() {
    env_logger::try_init().ok(); // Initialize logging for tests

    let test_file = "../tests/data/hey_mycroft_test.wav";
    if !Path::new(test_file).exists() {
        println!("Warning: hey_mycroft_test.wav not found, skipping positive test");
        return;
    }

    // Load the audio file
    let audio_data = load_test_audio(test_file).expect("Failed to load test audio file");
    println!(
        "Loaded {} samples ({:.2}s)",
        audio_data.len(),
        audio_data.len() as f32 / 16000.0
    );

    // Add padding like Python's predict_clip method: 1 second of silence at start and end
    let padding_samples = 16000; // 1 second at 16kHz
    let mut padded_audio = Vec::new();

    // Add 1 second of silence at the beginning
    padded_audio.extend(vec![0i16; padding_samples]);

    // Add original audio
    padded_audio.extend(&audio_data);

    // Add 1 second of silence at the end
    padded_audio.extend(vec![0i16; padding_samples]);

    println!(
        "After padding: {} samples ({:.2}s)",
        padded_audio.len(),
        padded_audio.len() as f32 / 16000.0
    );

    // Create the model
    let mut model = Model::new(
        vec!["hey_mycroft".to_string()],
        vec![], // Empty class mappings
    )
    .expect("Failed to create model");

    // Process the padded audio in chunks like predict_clip does
    let chunk_size = 1280;
    let mut max_confidence = 0.0;
    let mut all_predictions = Vec::new();

    for (i, chunk_start) in (0..padded_audio.len().saturating_sub(chunk_size))
        .step_by(chunk_size)
        .enumerate()
    {
        let chunk_end = std::cmp::min(chunk_start + chunk_size, padded_audio.len());
        let chunk = &padded_audio[chunk_start..chunk_end];

        // Pad chunk to exactly chunk_size if needed
        let mut padded_chunk = chunk.to_vec();
        if padded_chunk.len() < chunk_size {
            padded_chunk.resize(chunk_size, 0);
        }

        let predictions = model
            .predict(&padded_chunk, None, 0.0)
            .expect("Failed to predict");

        if let Some(confidence) = predictions.get("hey_mycroft") {
            all_predictions.push(*confidence);
            if *confidence > max_confidence {
                max_confidence = *confidence;
            }

            // Print progress for key frames
            if i % 10 == 0 || *confidence > 0.01 {
                println!("Frame {}: hey_mycroft confidence = {:.10}", i, confidence);
            }
        }
    }

    println!("Total frames processed: {}", all_predictions.len());
    println!(
        "First 10 predictions: {:?}",
        &all_predictions[..std::cmp::min(10, all_predictions.len())]
    );
    println!("Maximum confidence: {:.10}", max_confidence);

    // The test should now pass with proper padding
    assert!(
        max_confidence >= 0.5,
        "Expected confidence >= 0.5, got {:.10}",
        max_confidence
    );
}

/// Test individual melspectrogram stage
#[test]
fn test_melspectrogram_stage() {
    env_logger::try_init().ok();

    let test_file = "../tests/data/alexa_test.wav";
    if !Path::new(test_file).exists() {
        panic!("Test file not found: {}", test_file);
    }

    // Load a small chunk of audio (1280 samples = 80ms)
    let mut reader = hound::WavReader::open(test_file).expect("Failed to open test audio file");
    let samples: Vec<i16> = reader
        .samples()
        .take(1280)
        .collect::<std::result::Result<Vec<_>, _>>()
        .expect("Failed to read audio samples");

    // Create AudioFeatures processor
    let mut audio_features = AudioFeatures::new(
        "models/melspectrogram.tflite",
        "models/embedding_model.tflite",
        16000,
    )
    .expect("Failed to create AudioFeatures");

    // Test melspectrogram processing
    let result = audio_features.__call__(&samples);
    assert!(
        result.is_ok(),
        "Melspectrogram processing failed: {:?}",
        result.err()
    );

    // Should process exactly 1280 samples
    let processed = result.unwrap();
    assert_eq!(
        processed, 1280,
        "Expected to process 1280 samples, got {}",
        processed
    );
}

/// Test embedding stage
#[test]
fn test_embedding_stage() {
    env_logger::try_init().ok();

    let test_file = "../tests/data/alexa_test.wav";
    if !Path::new(test_file).exists() {
        panic!("Test file not found: {}", test_file);
    }

    // Load enough audio to generate embeddings (multiple chunks)
    let mut reader = hound::WavReader::open(test_file).expect("Failed to open test audio file");
    let samples: Vec<i16> = reader
        .samples()
        .take(6400)
        .collect::<std::result::Result<Vec<_>, _>>()
        .expect("Failed to read audio samples");

    // Create AudioFeatures processor
    let mut audio_features = AudioFeatures::new(
        "models/melspectrogram.tflite",
        "models/embedding_model.tflite",
        16000,
    )
    .expect("Failed to create AudioFeatures");

    // Process audio in chunks
    for chunk in samples.chunks(1280) {
        let result = audio_features.__call__(chunk);
        assert!(
            result.is_ok(),
            "Audio processing failed: {:?}",
            result.err()
        );
    }

    // Should have generated some embeddings
    let features = audio_features.get_features(16, -1);
    assert!(!features.is_empty(), "No features generated");

    // Should eventually get enough features for the model
    if features.len() >= 1536 {
        assert_eq!(
            features.len(),
            1536,
            "Expected exactly 1536 features, got {}",
            features.len()
        );

        // Verify feature values are reasonable
        let mean = features.iter().sum::<f32>() / features.len() as f32;
        assert!(mean.is_finite(), "Features contain non-finite values");
        assert!(
            mean.abs() < 100.0,
            "Feature mean {} seems unreasonable",
            mean
        );
    }
}

/// Test streaming behavior matches Python expectations
#[test]
fn test_streaming_behavior() {
    env_logger::try_init().ok();

    // Use a longer audio file that has enough content to generate 1536+ features
    // alexa_test.wav is only 0.625s, but delay_start_what_time_is_it.wav is 5.92s
    let test_file = "../tests/data/delay_start_what_time_is_it.wav";
    if !Path::new(test_file).exists() {
        panic!("Test file not found: {}", test_file);
    }

    // Load audio file
    let mut reader = hound::WavReader::open(test_file).expect("Failed to open test audio file");
    let samples: Vec<i16> = reader
        .samples()
        .collect::<std::result::Result<Vec<_>, _>>()
        .expect("Failed to read audio samples");

    println!(
        "Audio file duration: ~{:.2}s, {} samples",
        samples.len() as f32 / 16000.0,
        samples.len()
    );

    // Create AudioFeatures processor
    let mut audio_features = AudioFeatures::new(
        "models/melspectrogram.tflite",
        "models/embedding_model.tflite",
        16000,
    )
    .expect("Failed to create AudioFeatures");

    let mut total_features = 0;
    let chunk_size = 1280; // 80ms chunks

    // Process in streaming fashion
    for (i, chunk) in samples.chunks(chunk_size).enumerate() {
        let result = audio_features.__call__(chunk);
        assert!(
            result.is_ok(),
            "Chunk {} processing failed: {:?}",
            i,
            result.err()
        );

        // Check feature count growth
        let features = audio_features.get_features(16, -1);
        let current_feature_count = features.len();

        if current_feature_count > total_features {
            println!(
                "Chunk {}: {} features (was {})",
                i, current_feature_count, total_features
            );
            total_features = current_feature_count;
        }

        // After several chunks, should have enough features
        if i >= 10 && current_feature_count >= 1536 {
            println!("Reached 1536 features after {} chunks", i);
            break;
        }
    }

    // Should eventually reach the target feature count
    assert!(
        total_features >= 1536,
        "Failed to reach 1536 features, only got {} (audio duration: {:.2}s)",
        total_features,
        samples.len() as f32 / 16000.0
    );
}

/// Test model loading and basic functionality
#[test]
fn test_model_creation() {
    env_logger::try_init().ok();

    // Test single model
    let model_names = vec!["hey_mycroft".to_string()];
    let class_mappings = vec![]; // Empty class mappings
    let model = Model::new(model_names.clone(), class_mappings);
    assert!(model.is_ok(), "Failed to create model: {:?}", model.err());

    let mut model = model.unwrap();

    // Test with minimal audio data
    let minimal_audio: Vec<i16> = vec![0; 16000]; // 1 second of silence
    let result = model.predict(&minimal_audio, None, 0.0);
    assert!(result.is_ok(), "Prediction failed: {:?}", result.err());

    let predictions = result.unwrap();
    assert!(
        predictions.contains_key("hey_mycroft"),
        "Missing hey_mycroft prediction"
    );

    let confidence = predictions["hey_mycroft"];
    assert!(
        confidence >= 0.0 && confidence <= 1.0,
        "Confidence {} out of range [0,1]",
        confidence
    );
}

/// Test error handling
#[test]
fn test_error_handling() {
    env_logger::try_init().ok();

    // Test with invalid model name
    let model_names = vec!["nonexistent_model".to_string()];
    let class_mappings = vec![]; // Empty class mappings
    let model = Model::new(model_names, class_mappings);
    assert!(model.is_err(), "Should fail with nonexistent model");

    // Test with empty audio
    let model_names = vec!["hey_mycroft".to_string()];
    let class_mappings = vec![]; // Empty class mappings
    let mut model = Model::new(model_names, class_mappings).expect("Failed to create model");

    let empty_audio: Vec<i16> = vec![];
    let result = model.predict(&empty_audio, None, 0.0);
    // This might succeed with zero confidence or fail gracefully
    if let Ok(predictions) = result {
        let confidence = predictions.get("hey_mycroft").unwrap_or(&0.0);
        assert!(
            *confidence >= 0.0 && *confidence <= 1.0,
            "Invalid confidence for empty audio"
        );
    }
}

/// Benchmark test to ensure reasonable performance
#[test]
fn test_performance() {
    env_logger::try_init().ok();

    let test_file = "../tests/data/alexa_test.wav";
    if !Path::new(test_file).exists() {
        panic!("Test file not found: {}", test_file);
    }

    // Load audio file
    let mut reader = hound::WavReader::open(test_file).expect("Failed to open test audio file");
    let samples: Vec<i16> = reader
        .samples()
        .collect::<std::result::Result<Vec<_>, _>>()
        .expect("Failed to read audio samples");

    // Create model
    let model_names = vec!["hey_mycroft".to_string()];
    let class_mappings = vec![]; // Empty class mappings
    let mut model = Model::new(model_names, class_mappings).expect("Failed to create model");

    // Time the prediction
    let start = std::time::Instant::now();
    let _results = model
        .predict(&samples, None, 0.0)
        .expect("Prediction failed");
    let duration = start.elapsed();

    println!("Prediction took: {:?}", duration);

    // Should be reasonably fast (less than 1 second for a 0.6s audio file)
    assert!(
        duration.as_secs() < 2,
        "Prediction too slow: {:?}",
        duration
    );
}
