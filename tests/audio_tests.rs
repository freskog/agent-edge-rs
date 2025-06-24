use agent_edge_rs::audio::ChannelExtractor;

#[test]
fn test_channel_extractor_creation() {
    // Test valid channel extractor creation
    let extractor = ChannelExtractor::new(0, 6);
    assert!(extractor.is_ok());

    // Test invalid channel (out of range)
    let invalid_extractor = ChannelExtractor::new(6, 6);
    assert!(invalid_extractor.is_err());
}

#[test]
fn test_channel_extraction() {
    let extractor = ChannelExtractor::new(0, 6).unwrap();

    // Test with interleaved 6-channel audio (18 samples = 3 samples per channel)
    let interleaved = vec![
        1.0, 2.0, 3.0, 4.0, 5.0, 6.0, // First sample set
        7.0, 8.0, 9.0, 10.0, 11.0, 12.0, // Second sample set
        13.0, 14.0, 15.0, 16.0, 17.0, 18.0, // Third sample set
    ];

    let channel_0 = extractor.extract_channel(&interleaved);
    assert_eq!(channel_0, vec![1.0, 7.0, 13.0]);

    // Test channel 1 extraction
    let extractor_1 = ChannelExtractor::new(1, 6).unwrap();
    let channel_1 = extractor_1.extract_channel(&interleaved);
    assert_eq!(channel_1, vec![2.0, 8.0, 14.0]);
}

#[test]
fn test_channel_extractor_clone() {
    let extractor = ChannelExtractor::new(0, 6).unwrap();
    let cloned = extractor.clone();

    // Test that both extractors work the same way
    let test_data = vec![
        1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0,
    ];

    let result1 = extractor.extract_channel(&test_data);
    let result2 = cloned.extract_channel(&test_data);

    assert_eq!(result1, result2);
    assert_eq!(result1, vec![1.0, 7.0]);
}

#[test]
fn test_respeaker_channel_extraction() {
    // Test the specific ReSpeaker 4-mic array scenario
    let extractor = ChannelExtractor::new(0, 6).unwrap();

    // Simulate 6-channel interleaved audio from ReSpeaker
    // Channel layout: [0, 1, 2, 3, 4, 5, 0, 1, 2, 3, 4, 5, ...]
    let respeaker_data = vec![
        // First sample frame (6 channels)
        0.1, 0.2, 0.3, 0.4, 0.5, 0.6, // Second sample frame (6 channels)
        0.7, 0.8, 0.9, 1.0, 1.1, 1.2, // Third sample frame (6 channels)
        1.3, 1.4, 1.5, 1.6, 1.7, 1.8,
    ];

    let channel_0 = extractor.extract_channel(&respeaker_data);

    // Should extract samples from channel 0 (every 6th sample starting from index 0)
    assert_eq!(channel_0, vec![0.1, 0.7, 1.3]);
}

#[test]
fn test_i16_to_f32_conversion() {
    // Test i16 to f32 conversion (as used in PulseAudio capture)
    let i16_samples: Vec<i16> = vec![0, i16::MAX / 2, i16::MAX, i16::MIN];
    let f32_samples: Vec<f32> = i16_samples
        .iter()
        .map(|&sample| sample as f32 / i16::MAX as f32)
        .collect();

    assert_eq!(f32_samples[0], 0.0);
    assert!((f32_samples[1] - 0.5).abs() < 0.01);
    assert!((f32_samples[2] - 1.0).abs() < 0.01);
    assert!((f32_samples[3] - (-1.0)).abs() < 0.01);
}

#[test]
fn test_multi_channel_extraction() {
    // Test extracting different channels from the same data
    let six_channel_data = vec![
        10.0, 20.0, 30.0, 40.0, 50.0, 60.0, // Frame 1
        11.0, 21.0, 31.0, 41.0, 51.0, 61.0, // Frame 2
    ];

    let extractor_0 = ChannelExtractor::new(0, 6).unwrap();
    let extractor_5 = ChannelExtractor::new(5, 6).unwrap();

    assert_eq!(
        extractor_0.extract_channel(&six_channel_data),
        vec![10.0, 11.0]
    );
    assert_eq!(
        extractor_5.extract_channel(&six_channel_data),
        vec![60.0, 61.0]
    );
}

#[test]
fn test_empty_audio_handling() {
    let extractor = ChannelExtractor::new(0, 6).unwrap();
    let empty_data: Vec<f32> = vec![];

    let result = extractor.extract_channel(&empty_data);
    assert_eq!(result, vec![]);
}
