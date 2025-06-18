use agent_edge_rs::audio::{AudioCapture, AudioCaptureConfig, ChannelExtractor};
use std::time::Duration;

#[test]
fn test_audio_capture_config_with_buffer_size() {
    let config = AudioCaptureConfig {
        sample_rate: 48000,
        channels: 2,
        device_name: Some("test-device".to_string()),
        buffer_size: 2048,
        target_latency_ms: 50,
    };
    
    assert_eq!(config.sample_rate, 48000);
    assert_eq!(config.channels, 2);
    assert_eq!(config.device_name, Some("test-device".to_string()));
    assert_eq!(config.buffer_size, 2048);
}

#[test]
fn test_audio_capture_creation() {
    let config = AudioCaptureConfig::default();
    
    // Creating an AudioCapture should succeed (even without real audio hardware)
    let result = AudioCapture::new(config);
    assert!(result.is_ok());
}

#[test]
fn test_channel_extractor_clone() {
    let extractor = ChannelExtractor::new(0, 6).unwrap();
    let cloned = extractor.clone();
    
    // Test that both extractors work the same way
    let test_data = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0];
    
    let result1 = extractor.extract_channel(&test_data);
    let result2 = cloned.extract_channel(&test_data);
    
    assert_eq!(result1, result2);
    assert_eq!(result1, vec![1.0, 7.0]);
}

#[test]
fn test_audio_capture_list_devices() {
    let config = AudioCaptureConfig::default();
    let audio_capture = AudioCapture::new(config).unwrap();
    
    // Listing devices should not fail (even if no devices are available)
    let result = audio_capture.list_input_devices();
    assert!(result.is_ok());
    
    // The result should be a vector (might be empty on systems without audio)
    let devices = result.unwrap();
    println!("Available audio devices: {:?}", devices);
}

#[test] 
fn test_respeaker_channel_extraction() {
    // Test the specific ReSpeaker 4-mic array scenario
    let extractor = ChannelExtractor::new(0, 6).unwrap();
    
    // Simulate 6-channel interleaved audio from ReSpeaker
    // Channel layout: [0, 1, 2, 3, 4, 5, 0, 1, 2, 3, 4, 5, ...]
    let respeaker_data = vec![
        // First sample frame (6 channels)
        0.1, 0.2, 0.3, 0.4, 0.5, 0.6,
        // Second sample frame (6 channels)  
        0.7, 0.8, 0.9, 1.0, 1.1, 1.2,
        // Third sample frame (6 channels)
        1.3, 1.4, 1.5, 1.6, 1.7, 1.8,
    ];
    
    let channel_0 = extractor.extract_channel(&respeaker_data);
    
    // Should extract samples from channel 0 (every 6th sample starting from index 0)
    assert_eq!(channel_0, vec![0.1, 0.7, 1.3]);
}

#[test]
fn test_audio_sample_format_conversion() {
    // Test i16 to f32 conversion (as used in audio capture)
    let i16_samples: Vec<i16> = vec![0, i16::MAX / 2, i16::MAX, i16::MIN];
    let f32_samples: Vec<f32> = i16_samples.iter()
        .map(|&sample| sample as f32 / i16::MAX as f32)
        .collect();
    
    assert_eq!(f32_samples[0], 0.0);
    assert!((f32_samples[1] - 0.5).abs() < 0.01);
    assert!((f32_samples[2] - 1.0).abs() < 0.01);
    assert!((f32_samples[3] - (-1.0)).abs() < 0.01);
}

#[test]
fn test_u16_to_f32_conversion() {
    // Test u16 to f32 conversion (as used in audio capture)
    let u16_samples: Vec<u16> = vec![0, u16::MAX / 2, u16::MAX];
    let f32_samples: Vec<f32> = u16_samples.iter()
        .map(|&sample| (sample as f32 - u16::MAX as f32 / 2.0) / (u16::MAX as f32 / 2.0))
        .collect();
    
    assert!((f32_samples[0] - (-1.0)).abs() < 0.01);
    assert!((f32_samples[1] - 0.0).abs() < 0.01);
    assert!((f32_samples[2] - 1.0).abs() < 0.01);
}

// Note: The following test requires actual audio hardware and cannot be run in CI
// It's included for manual testing on Raspberry Pi
#[test]
#[ignore]
fn test_audio_capture_integration() {
    let config = AudioCaptureConfig {
        sample_rate: 16000,
        channels: 6, // ReSpeaker
        device_name: None, // Use default
        buffer_size: 1024,
        target_latency_ms: 50,
    };
    
    let mut audio_capture = AudioCapture::new(config).unwrap();
    
    // Start capture
    audio_capture.start().unwrap();
    
    // Capture for a short time
    let start = std::time::Instant::now();
    let mut buffer_count = 0;
    
    while start.elapsed() < Duration::from_secs(2) {
        if let Ok(Some(buffer)) = audio_capture.try_get_audio_buffer() {
            buffer_count += 1;
            assert!(!buffer.is_empty());
            
            // Check that samples are within expected range
            for &sample in &buffer {
                assert!(sample >= -1.0 && sample <= 1.0);
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    
    audio_capture.stop().unwrap();
    
    // Should have captured some audio
    assert!(buffer_count > 0);
    println!("Captured {} audio buffers", buffer_count);
} 