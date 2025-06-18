use agent_edge_rs::{EdgeError, Result};
use agent_edge_rs::audio::{AudioCaptureConfig, ChannelExtractor};
use agent_edge_rs::detection::DetectionPipeline;

#[test]
fn test_error_types() {
    let audio_error = EdgeError::Audio("Test audio error".to_string());
    let model_error = EdgeError::Model("Test model error".to_string());
    let detection_error = EdgeError::Detection("Test detection error".to_string());
    
    assert!(format!("{}", audio_error).contains("Audio error"));
    assert!(format!("{}", model_error).contains("Model error"));
    assert!(format!("{}", detection_error).contains("Detection error"));
}

#[test]
fn test_result_type() {
    let success: Result<String> = Ok("success".to_string());
    let failure: Result<String> = Err(EdgeError::Config("test error".to_string()));
    
    assert!(success.is_ok());
    assert!(failure.is_err());
}

#[test]
fn test_audio_capture_config() {
    let default_config = AudioCaptureConfig::default();
    assert_eq!(default_config.sample_rate, 16000);
    assert_eq!(default_config.channels, 6);
    assert_eq!(default_config.device_name, None);
    
    let custom_config = AudioCaptureConfig {
        sample_rate: 48000,
        channels: 2,
        device_name: Some("test-device".to_string()),
        buffer_size: 2048,
        target_latency_ms: 50,
    };
    assert_eq!(custom_config.sample_rate, 48000);
    assert_eq!(custom_config.channels, 2);
    assert_eq!(custom_config.device_name, Some("test-device".to_string()));
}

#[test]
fn test_channel_extractor() {
    // Test valid channel extractor creation
    let extractor = ChannelExtractor::new(0, 6);
    assert!(extractor.is_ok());
    
    // Test invalid channel (out of range)
    let invalid_extractor = ChannelExtractor::new(6, 6);
    assert!(invalid_extractor.is_err());
    match invalid_extractor {
        Err(EdgeError::Audio(msg)) => assert!(msg.contains("out of range")),
        _ => panic!("Expected Audio error"),
    }
}

#[test]
fn test_channel_extraction() {
    let extractor = ChannelExtractor::new(0, 6).unwrap();
    
    // Test with interleaved 6-channel audio (18 samples = 3 samples per channel)
    let interleaved = vec![
        1.0, 2.0, 3.0, 4.0, 5.0, 6.0,  // First sample set
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
fn test_detection_pipeline() {
    // Test valid threshold
    let pipeline = DetectionPipeline::new(0.8).unwrap();
    
    // Test invalid thresholds
    assert!(DetectionPipeline::new(-0.1).is_err());
    assert!(DetectionPipeline::new(1.1).is_err());
    
    // Test processing (should return false for now)
    let samples = vec![0.1, 0.2, 0.3, 0.4, 0.5];
    let result = pipeline.process_audio(&samples).unwrap();
    assert_eq!(result, false); // No detection in Phase 1
} 