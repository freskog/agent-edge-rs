#[test]
fn test_i16_to_f32_conversion() {
    // Test i16 to f32 conversion (as used in audio processing)
    let i16_samples: Vec<i16> = vec![0, i16::MAX / 2, i16::MAX, i16::MIN];
    let f32_samples: Vec<f32> = i16_samples
        .iter()
        .map(|&sample| sample as f32 / 32768.0) // Normalize to -1.0 to 1.0
        .collect();

    assert_eq!(f32_samples[0], 0.0);
    assert!((f32_samples[1] - 0.5).abs() < 0.01);
    assert!((f32_samples[2] - 0.999969).abs() < 0.01); // Close to 1.0 but not exactly
    assert!((f32_samples[3] - (-1.0)).abs() < 0.01);
}

#[test]
fn test_channel_extraction_logic() {
    // Test the channel extraction logic that's used in the audio capture
    let channels = 6;
    let target_channel = 0;

    // Simulate interleaved 6-channel audio
    let interleaved = vec![
        1, 2, 3, 4, 5, 6, // Frame 1
        7, 8, 9, 10, 11, 12, // Frame 2
        13, 14, 15, 16, 17, 18, // Frame 3
    ];

    // Extract channel 0 (mimicking the audio capture logic)
    let channel_0: Vec<i16> = interleaved
        .chunks(channels)
        .filter_map(|chunk| chunk.get(target_channel).copied())
        .collect();

    assert_eq!(channel_0, vec![1, 7, 13]);

    // Test channel 5 extraction
    let target_channel = 5;
    let channel_5: Vec<i16> = interleaved
        .chunks(channels)
        .filter_map(|chunk| chunk.get(target_channel).copied())
        .collect();

    assert_eq!(channel_5, vec![6, 12, 18]);
}

#[test]
fn test_chunk_size_calculation() {
    // Test that we're calculating the expected chunk size correctly
    let sample_rate = 16000;
    let duration_ms = 80; // 80ms chunks as expected by detection pipeline
    let expected_samples = (sample_rate / 1000) * duration_ms;

    assert_eq!(expected_samples, 1280); // This is what the detection pipeline expects
}

#[test]
fn test_empty_audio_handling() {
    let empty_data: Vec<i16> = vec![];

    // Test that empty data doesn't crash channel extraction
    let channels = 6;
    let target_channel = 0;
    let result = empty_data
        .chunks(channels)
        .filter_map(|chunk| chunk.get(target_channel).copied())
        .collect::<Vec<i16>>();

    assert_eq!(result, Vec::<i16>::new());
}
