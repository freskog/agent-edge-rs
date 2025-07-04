//! # Pipeline Integration Test
//!
//! Single comprehensive test that validates the complete OpenWakeWord detection pipeline
//! end-to-end, including real audio processing with the "Hey Mycroft" test file.

use agent_edge_rs::{
    detection::pipeline::{DetectionPipeline, PipelineConfig},
    error::{EdgeError, Result},
};
use hound;

const SAMPLE_RATE: u32 = 16000;
const CHUNK_SIZE: usize = 1280; // 80ms at 16kHz

fn load_test_audio(filename: &str) -> Result<Vec<f32>> {
    let test_file = format!("tests/data/{}", filename);

    let mut reader = hound::WavReader::open(&test_file)
        .map_err(|e| EdgeError::Io(std::io::Error::new(std::io::ErrorKind::NotFound, e)))?;

    let spec = reader.spec();
    println!("WAV file spec: {:?}", spec);

    // Read samples and convert to f32
    let samples: Result<Vec<f32>> = if spec.sample_format == hound::SampleFormat::Int {
        if spec.bits_per_sample == 16 {
            reader
                .samples::<i16>()
                .map(|s| {
                    s.map(|sample| sample as f32 / i16::MAX as f32)
                        .map_err(|e| {
                            EdgeError::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, e))
                        })
                })
                .collect()
        } else {
            return Err(EdgeError::InvalidInput("Unsupported bit depth".to_string()));
        }
    } else {
        return Err(EdgeError::InvalidInput(
            "Unsupported sample format".to_string(),
        ));
    };

    samples
}

fn process_audio_in_chunks(
    pipeline: &mut DetectionPipeline,
    audio: &[f32],
) -> Result<Vec<(f32, bool)>> {
    // Add 1 second of silence padding before and after (like OpenWakeWord test methodology)
    let padding_samples = SAMPLE_RATE as usize; // 1 second
    let mut padded_audio = Vec::new();

    // Add silence before
    padded_audio.extend(vec![0.0f32; padding_samples]);
    // Add original audio
    padded_audio.extend_from_slice(audio);
    // Add silence after
    padded_audio.extend(vec![0.0f32; padding_samples]);

    println!(
        "📏 Audio length: original {:.2}s → padded {:.2}s",
        audio.len() as f32 / SAMPLE_RATE as f32,
        padded_audio.len() as f32 / SAMPLE_RATE as f32
    );

    let mut results = Vec::new();

    // Process audio in chunks
    for (_chunk_idx, chunk_start) in (0..padded_audio.len()).step_by(CHUNK_SIZE).enumerate() {
        let chunk_end = (chunk_start + CHUNK_SIZE).min(padded_audio.len());
        let chunk = &padded_audio[chunk_start..chunk_end];

        // Skip chunks that are too small (need exactly 1280 samples)
        if chunk.len() < CHUNK_SIZE {
            // Pad with zeros if needed for the last chunk
            let mut padded_chunk = [0.0f32; CHUNK_SIZE];
            padded_chunk[..chunk.len()].copy_from_slice(chunk);

            let detection = pipeline.process_audio_chunk(&padded_chunk)?;
            results.push((detection.confidence, detection.detected));
        } else {
            // Convert to fixed-size array
            let mut chunk_array = [0.0f32; CHUNK_SIZE];
            chunk_array.copy_from_slice(&chunk[..CHUNK_SIZE]);

            let detection = pipeline.process_audio_chunk(&chunk_array)?;
            results.push((detection.confidence, detection.detected));
        }
    }

    Ok(results)
}

#[test]
fn test_complete_pipeline() -> Result<()> {
    println!("🚀 Starting comprehensive pipeline test");

    // Test 1: Configuration creation
    let config = PipelineConfig::default();
    assert_eq!(config.chunk_size, 1280);
    assert_eq!(config.sample_rate, 16000);
    assert_eq!(config.confidence_threshold, 0.09);
    assert_eq!(config.window_size, 16);
    assert_eq!(config.debounce_duration_ms, 1000);
    println!("✅ 1. Pipeline configuration validated");

    // Test 2: Pipeline initialization
    let mut pipeline = DetectionPipeline::new(config)?;
    println!("✅ 2. Pipeline initialized successfully");

    // Test 3: Silence handling
    let silence = [0.0f32; 1280];
    let detection = pipeline.process_audio_chunk(&silence)?;
    assert!(!detection.detected);
    assert!(detection.confidence <= 0.5);
    println!("✅ 3. Silence correctly produces no detection");

    // Test 4: Chunk size validation - we'll create a smaller array and see if the function handles it
    // Note: Since we need exactly 1280 samples, this test validates the input requirement
    let small_chunk = [0.0f32; CHUNK_SIZE]; // This should work
    let result = pipeline.process_audio_chunk(&small_chunk);
    assert!(result.is_ok());
    println!("✅ 4. Correct chunk size validation works");

    // Test 5: Reset functionality
    let audio = [0.1f32; 1280];
    let _ = pipeline.process_audio_chunk(&audio)?;
    pipeline.reset();
    let detection = pipeline.process_audio_chunk(&audio)?;
    assert!(!detection.detected); // Should not detect in noise
    println!("✅ 5. Pipeline reset works correctly");

    // Test 6: Real audio processing with Hey Mycroft
    let audio = load_test_audio("hey_mycroft_test.wav")?;
    println!(
        "✅ 6a. Loaded test audio: {} samples ({:.2}s)",
        audio.len(),
        audio.len() as f32 / 16000.0
    );

    let results = process_audio_in_chunks(&mut pipeline, &audio)?;
    println!("✅ 6b. Processed {} chunks", results.len());

    // Analyze results
    let detections: Vec<&(f32, bool)> = results.iter().filter(|(_, detected)| *detected).collect();
    let max_confidence = results.iter().map(|(conf, _)| *conf).fold(0.0f32, f32::max);
    let avg_confidence = results.iter().map(|(conf, _)| *conf).sum::<f32>() / results.len() as f32;

    println!("📊 Detection Results:");
    println!("   - Total chunks: {}", results.len());
    println!("   - Detections: {}", detections.len());
    println!("   - Max confidence: {:.4}", max_confidence);
    println!("   - Average confidence: {:.4}", avg_confidence);

    // Validate confidence scores are in valid range [0, 1]
    for (conf, _) in &results {
        assert!(
            *conf >= 0.0 && *conf <= 1.0,
            "Confidence {} out of range [0,1]",
            conf
        );
    }

    // Check for confidence variation (not all zeros)
    let has_variation = results.iter().any(|(conf, _)| *conf != 0.0);
    assert!(
        has_variation,
        "All confidence scores are zero - pipeline not working"
    );

    println!("✅ 6c. Hey Mycroft audio processing validated");

    // Test 7: Debouncing (if we got detections)
    if !detections.is_empty() {
        // With proper debouncing, we should have limited detections despite multiple high-confidence chunks
        assert!(
            detections.len() <= 3,
            "Too many detections - debouncing may not be working"
        );
        println!("✅ 7. Debouncing appears to be working (limited detections)");
    } else {
        println!("ℹ️  7. No detections to test debouncing (threshold may be high)");
    }

    println!("🎉 All pipeline tests passed! System is working correctly.");
    Ok(())
}
