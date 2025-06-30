use agent_edge_rs::audio_capture::AudioCaptureConfig;

#[test]
fn test_audio_config_default() {
    let config = AudioCaptureConfig::default();
    assert_eq!(config.channel_capacity, 100);
    assert_eq!(config.chunk_size, 256);
    assert_eq!(config.device_name, None);
    assert_eq!(config.target_channel, 0);
    assert_eq!(config.target_latency_ms, 30);
}

#[test]
fn test_audio_config_custom() {
    let config = AudioCaptureConfig {
        channel_capacity: 200,
        chunk_size: 512,
        device_name: Some("test-device".to_string()),
        target_channel: 1,
        target_latency_ms: 50,
    };

    assert_eq!(config.channel_capacity, 200);
    assert_eq!(config.chunk_size, 512);
    assert_eq!(config.device_name, Some("test-device".to_string()));
    assert_eq!(config.target_channel, 1);
    assert_eq!(config.target_latency_ms, 50);
}

#[test]
fn test_audio_config_clone() {
    let config = AudioCaptureConfig {
        channel_capacity: 150,
        chunk_size: 1024,
        device_name: Some("test-device".to_string()),
        target_channel: 2,
        target_latency_ms: 40,
    };

    let cloned = config.clone();
    assert_eq!(config.channel_capacity, cloned.channel_capacity);
    assert_eq!(config.chunk_size, cloned.chunk_size);
    assert_eq!(config.device_name, cloned.device_name);
    assert_eq!(config.target_channel, cloned.target_channel);
    assert_eq!(config.target_latency_ms, cloned.target_latency_ms);
}

#[test]
fn test_respeaker_config() {
    // Test configuration suitable for ReSpeaker 4-mic array
    let config = AudioCaptureConfig {
        channel_capacity: 100,
        chunk_size: 256,
        device_name: Some("respeaker-4-mic".to_string()),
        target_channel: 0,
        target_latency_ms: 30,
    };

    // Note: The actual channel handling is now done in the PulseAudio/CPAL implementation
    // rather than in the config struct
    assert_eq!(config.device_name, Some("respeaker-4-mic".to_string()));
}

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

#[test]
fn test_wakeword_optimized_config() {
    let config = AudioCaptureConfig {
        channel_capacity: 100,
        chunk_size: 256, // 16ms chunks optimal for wakeword detection
        device_name: None,
        target_channel: 0,
        target_latency_ms: 30,
    };

    assert_eq!(config.channel_capacity, 100);
    assert_eq!(config.chunk_size, 256);
    assert_eq!(config.device_name, None);
    assert_eq!(config.target_channel, 0);
    assert_eq!(config.target_latency_ms, 30);
}
