use agent_edge_rs::audio_capture::{AudioCapture, AudioCaptureConfig, PlatformAudioCapture};
use env_logger;
use log;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    // Create configuration
    let config = AudioCaptureConfig {
        sample_rate: 16000,
        channels: 1,
        target_channel: 0,
        device_name: None,
        target_latency_ms: 50,
        app_name: "audio-demo".to_string(),
        stream_name: "demo-capture".to_string(),
    };

    log::info!("🎵 Audio Capture Demo - Platform-Specific Implementation");

    // Show platform and backend
    #[cfg(target_os = "linux")]
    log::info!("🐧 Platform: Linux → PulseAudio backend");
    #[cfg(target_os = "macos")]
    log::info!("🍎 Platform: macOS → CPAL backend");
    #[cfg(target_os = "windows")]
    log::info!("🪟 Platform: Windows → CPAL backend");

    log::info!("Config: {:?}", config);

    // Create audio capture - will use the best available backend
    let mut capture = match PlatformAudioCapture::new(config) {
        Ok(capture) => {
            log::info!("✅ Audio capture created successfully");
            capture
        }
        Err(e) => {
            log::error!("❌ Failed to create audio capture: {}", e);
            return Err(e.into());
        }
    };

    // Show device information
    match capture.list_devices() {
        Ok(devices) => {
            log::info!("📱 Available audio devices:");
            for device in devices {
                log::info!(
                    "  - {} ({}): {} channels, rates: {:?}",
                    device.name,
                    if device.is_default {
                        "default"
                    } else {
                        "non-default"
                    },
                    device.max_channels,
                    device.supported_sample_rates
                );
            }
        }
        Err(e) => log::warn!("Could not list devices: {}", e),
    }

    // Test 1: Basic start/stop
    log::info!("🚀 Test 1: Basic start/stop");
    capture.start()?;
    log::info!("Capture started, active: {}", capture.is_active());

    // Let it run for a moment
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    log::info!("Available samples: {}", capture.available_samples());

    capture.stop()?;
    log::info!("Capture stopped, active: {}", capture.is_active());

    // Test 2: Read some chunks
    log::info!("📊 Test 2: Reading audio chunks");
    capture.start()?;

    for i in 0..3 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        match capture.read_chunk() {
            Ok(chunk) => {
                log::info!("Chunk {}: {} samples", i + 1, chunk.len());
                if !chunk.is_empty() {
                    let avg = chunk.iter().map(|&x| x as f64).sum::<f64>() / chunk.len() as f64;
                    let rms = (chunk.iter().map(|&x| (x as f64).powi(2)).sum::<f64>()
                        / chunk.len() as f64)
                        .sqrt();
                    log::info!("  Average: {:.1}, RMS: {:.1}", avg, rms);
                }
            }
            Err(e) => log::warn!("Failed to read chunk {}: {}", i + 1, e),
        }
    }

    capture.stop()?;

    // Test 3: Record for duration
    log::info!("⏱️  Test 3: Record for 2 seconds");
    let samples = capture.record_for_duration(2.0).await?;
    log::info!("Recorded {} samples in 2 seconds", samples.len());
    log::info!(
        "Expected ~{} samples at {} Hz",
        2.0 * capture.config().sample_rate as f32,
        capture.config().sample_rate
    );

    // Show statistics
    let stats = capture.get_stats();
    log::info!("📈 Audio Statistics:");
    log::info!("  Total samples: {}", stats.total_samples_captured);
    log::info!("  Sample rate: {} Hz", stats.current_sample_rate);
    log::info!("  Channels: {}", stats.current_channels);
    log::info!("  Buffer underruns: {}", stats.buffer_underruns);
    log::info!("  Buffer overruns: {}", stats.buffer_overruns);

    log::info!("✅ Audio capture demo completed successfully!");

    Ok(())
}
