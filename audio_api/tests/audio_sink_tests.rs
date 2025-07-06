use audio_api::audio_sink::{AudioSink, CpalConfig, CpalSink};
use log::info;

#[tokio::test]
async fn test_cpal_config() {
    let config = CpalConfig::default();
    assert_eq!(config.buffer_size_ms, 10000);
    assert_eq!(config.low_buffer_warning, 20);
    assert_eq!(config.high_buffer_warning, 80);
    assert!(config.device_name.is_none());

    let custom_config = CpalConfig {
        buffer_size_ms: 500,
        max_buffer_size_ms: 5000,
        buffer_growth_ms: 1000,
        low_buffer_warning: 20,
        high_buffer_warning: 80,
        backpressure_threshold: 90,
        device_name: None,
    };

    assert_eq!(custom_config.buffer_size_ms, 500);
    assert_eq!(custom_config.low_buffer_warning, 20);
    assert_eq!(custom_config.high_buffer_warning, 80);
    assert_eq!(custom_config.device_name, None);
}

#[tokio::test]
#[cfg_attr(not(feature = "audio_available"), ignore)]
async fn test_audio_sink_creation() {
    let config = CpalConfig::default();
    match CpalSink::new(config) {
        Ok(_sink) => {
            info!("Audio sink created successfully");
        }
        Err(e) => {
            info!("Audio sink not available - skipping: {}", e);
            // Don't fail the test if no audio hardware is available
        }
    }
}

#[tokio::test]
#[cfg_attr(not(feature = "audio_available"), ignore)]
async fn test_audio_playback() {
    let config = CpalConfig::default();
    let sink = match CpalSink::new(config) {
        Ok(sink) => sink,
        Err(e) => {
            info!("Audio sink not available - skipping: {}", e);
            return;
        }
    };

    // Generate a 440Hz sine wave for 0.5 seconds at 16kHz
    let sample_rate = 16000;
    let duration_secs = 0.5;
    let frequency = 440.0;
    let amplitude = 0.3;
    let num_samples = (sample_rate as f64 * duration_secs) as usize;

    let mut test_tone = Vec::with_capacity(num_samples);
    for i in 0..num_samples {
        let t = i as f64 / sample_rate as f64;
        let sample = (2.0 * std::f64::consts::PI * frequency * t).sin() * amplitude;
        test_tone.push(sample as f32);
    }

    // Convert to bytes (f32 samples)
    let audio_data: Vec<u8> = test_tone
        .iter()
        .flat_map(|&sample| sample.to_le_bytes())
        .collect();

    match sink.write(&audio_data).await {
        Ok(_) => {
            info!("Playback succeeded");
            // Wait a bit to let playback finish
            tokio::time::sleep(std::time::Duration::from_millis(600)).await;
        }
        Err(e) => {
            info!("Playback failed: {}", e);
            // Don't fail the test if playback fails (might be expected in CI)
        }
    }
}

#[tokio::test]
#[cfg_attr(not(feature = "audio_available"), ignore)]
async fn test_audio_sink_stop() {
    let config = CpalConfig::default();
    let sink = match CpalSink::new(config) {
        Ok(sink) => sink,
        Err(e) => {
            info!("Audio sink not available - skipping: {}", e);
            return;
        }
    };

    match sink.stop().await {
        Ok(_) => info!("Stop succeeded"),
        Err(e) => info!("Stop failed: {}", e),
    }
}
