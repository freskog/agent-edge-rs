use audio_api::audio_sink::{AudioSink, CpalConfig};
use audio_api::platform::AudioPlatform;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    println!("🧪 Testing AudioSink platform conversion functionality...");

    // Create AudioSink with RaspberryPi platform (expects 44.1kHz output)
    let config = CpalConfig::default();
    let sink = AudioSink::new_with_platform(config, AudioPlatform::RaspberryPi)?;

    // Generate 1 second of 44.1kHz s16le test data (sine wave) - TTS output format
    let mut test_data = Vec::new();
    let sample_rate = 44100;
    let duration = 1.0; // 1 second
    let frequency = 440.0; // A4 note

    for i in 0..(sample_rate as f64 * duration) as usize {
        let t = i as f64 / sample_rate as f64;
        let sample = (frequency * 2.0 * std::f64::consts::PI * t).sin();
        let i16_sample = (sample * 32767.0) as i16;
        test_data.extend_from_slice(&i16_sample.to_le_bytes());
    }

    println!(
        "📊 Generated {} bytes of 44.1kHz s16le test data",
        test_data.len()
    );
    println!(
        "📊 This represents {} samples at 44.1kHz",
        test_data.len() / 2
    );

    // Write the test data - this should just convert from s16le to f32 (no resampling)
    println!("🔄 Writing test data to AudioSink (should convert s16le -> f32, no resampling)...");
    sink.write_chunk(test_data).await?;

    // Wait for completion
    println!("⏰ Waiting for playback to complete...");
    sink.end_stream_and_wait().await?;

    println!("✅ Platform conversion test completed successfully!");
    println!("📝 Check the logs above for platform conversion debug messages");

    Ok(())
}
