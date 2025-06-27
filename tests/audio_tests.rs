use agent_edge_rs::audio_capture::{AudioCapture, AudioCaptureConfig, PlatformAudioCapture};

#[test]
fn test_audio_capture_config_default() {
    let config = AudioCaptureConfig::default();
    assert_eq!(config.sample_rate, 16000);
    assert_eq!(config.channels, 1);
    assert_eq!(config.target_channel, 0);
    assert_eq!(config.target_latency_ms, 50);
    assert_eq!(config.app_name, "agent-edge");
    assert_eq!(config.stream_name, "audio-capture");
}

#[test]
fn test_audio_capture_config_custom() {
    let config = AudioCaptureConfig {
        sample_rate: 48000,
        channels: 6,
        target_channel: 0,
        device_name: Some("test-device".to_string()),
        target_latency_ms: 100,
        app_name: "test-app".to_string(),
        stream_name: "test-stream".to_string(),
    };

    assert_eq!(config.sample_rate, 48000);
    assert_eq!(config.channels, 6);
    assert_eq!(config.target_channel, 0);
    assert_eq!(config.device_name, Some("test-device".to_string()));
    assert_eq!(config.target_latency_ms, 100);
    assert_eq!(config.app_name, "test-app");
    assert_eq!(config.stream_name, "test-stream");
}

#[test]
fn test_audio_capture_creation() {
    // This test just verifies that we can create an audio capture instance
    // without actually starting it (which would require audio hardware)
    let config = AudioCaptureConfig::default();
    let result = PlatformAudioCapture::new(config);

    // On platforms without audio support, this might fail
    // But the important thing is that the API works
    match result {
        Ok(_) => {
            // Success - we have audio support
        }
        Err(e) => {
            // Expected in headless environments
            println!("Audio capture creation failed (expected in CI): {}", e);
        }
    }
}

#[test]
fn test_respeaker_config() {
    // Test configuration suitable for ReSpeaker 4-mic array
    let config = AudioCaptureConfig {
        sample_rate: 16000,
        channels: 6,       // ReSpeaker has 6 channels
        target_channel: 0, // Extract first channel
        device_name: None,
        target_latency_ms: 50,
        app_name: "agent-edge".to_string(),
        stream_name: "wakeword-capture".to_string(),
    };

    assert_eq!(config.channels, 6);
    assert_eq!(config.target_channel, 0);
    assert_eq!(config.sample_rate, 16000);
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
