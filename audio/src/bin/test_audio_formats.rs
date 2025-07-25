use audio::platform::{AudioPlatform, PlatformSampleFormat};
use clap::Parser;

#[derive(Parser)]
#[command(name = "test_audio_formats")]
#[command(about = "Test audio format handling and platform detection")]
struct Args {
    /// Force platform detection (raspberry-pi or macos)
    #[arg(short, long)]
    platform: Option<String>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let args = Args::parse();

    println!("🔍 Audio Format Test");
    println!("==================");

    // Test 1: Platform Detection
    test_platform_detection(args.platform.as_deref())?;

    // Test 2: Audio Buffer Creation Logic
    test_audio_buffer_logic()?;

    // Test 3: Format Conversion Test
    test_format_conversion()?;

    println!("\n✅ Audio format tests completed!");
    Ok(())
}

fn test_platform_detection(force_platform: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n🎯 Test 1: Platform Detection");
    println!("-----------------------------");

    // Build-time platform detection (what the code actually uses)
    #[cfg(target_os = "macos")]
    let detected_platform = AudioPlatform::MacOS;
    #[cfg(target_os = "linux")]
    let detected_platform = AudioPlatform::RaspberryPi;

    let platform = if let Some(force) = force_platform {
        match force {
            "macos" => AudioPlatform::MacOS,
            "raspberry-pi" => AudioPlatform::RaspberryPi,
            _ => {
                println!(
                    "❌ Invalid platform: {}. Use 'macos' or 'raspberry-pi'",
                    force
                );
                return Ok(());
            }
        }
    } else {
        detected_platform
    };

    println!("📱 Detected Platform: {:?}", platform);

    // Get platform configurations
    let capture_config = platform.capture_config();
    let playback_config = platform.playback_config();
    let stt_format = platform.stt_format();
    let tts_format = platform.tts_format();

    println!("🎤 Capture Config:");
    println!("   Sample Rate: {}Hz", capture_config.preferred_sample_rate);
    println!("   Format: {:?}", capture_config.preferred_format);
    println!("   Channels: {}", capture_config.channel_count);
    println!("   Description: {}", capture_config.description);

    println!("🔊 Playback Config:");
    println!("   Sample Rate: {}Hz", playback_config.sample_rate);
    println!("   Format: {:?}", playback_config.format);
    println!("   Channels: {}", playback_config.channels);
    println!("   Description: {}", playback_config.description);

    println!(
        "🎧 STT Format: {}Hz, {:?}, {} channels",
        stt_format.sample_rate, stt_format.format, stt_format.channels
    );
    println!(
        "🗣️  TTS Format: {}Hz, {:?}, {} channels",
        tts_format.sample_rate, tts_format.format, tts_format.channels
    );

    Ok(())
}

fn test_audio_buffer_logic() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n🎯 Test 2: Audio Buffer Creation Logic");
    println!("--------------------------------------");

    // Build-time platform detection
    #[cfg(target_os = "macos")]
    let platform = AudioPlatform::MacOS;
    #[cfg(target_os = "linux")]
    let platform = AudioPlatform::RaspberryPi;

    let playback_config = platform.playback_config();

    println!("📋 Platform: {:?}", platform);
    println!("📋 Platform wants: {:?}", playback_config.format);

    // Simulate hardware detection
    // On Pi, hardware often returns F32 even though platform prefers I16
    let simulated_hardware_formats = vec![
        ("Pi with USB DAC", cpal::SampleFormat::F32),
        ("Pi with I2S DAC", cpal::SampleFormat::I16),
        ("Mac built-in", cpal::SampleFormat::F32),
    ];

    for (hardware_name, hardware_format) in simulated_hardware_formats {
        println!("\n🔧 Testing: {}", hardware_name);
        println!("   Hardware supports: {:?}", hardware_format);

        // This is the logic from audio_sink.rs
        let use_format = match (&playback_config.format, hardware_format) {
            (PlatformSampleFormat::I16, cpal::SampleFormat::I16) => {
                println!("   ✅ Perfect match: using I16 format (no conversions)");
                cpal::SampleFormat::I16
            }
            (PlatformSampleFormat::F32, _) => {
                println!("   ✅ Using F32 format as preferred by platform");
                cpal::SampleFormat::F32
            }
            (PlatformSampleFormat::I16, _) => {
                println!("   ⚠️  Hardware doesn't support I16, falling back to F32");
                cpal::SampleFormat::F32
            }
        };

        // This is where the bug WAS - the buffer creation logic
        // OLD BUG: Always used platform preference
        let old_buffer_type = match playback_config.format {
            PlatformSampleFormat::I16 => {
                println!("   🔊 OLD: Using I16 audio buffer (platform preference)");
                "I16 Buffer"
            }
            PlatformSampleFormat::F32 => {
                println!("   🔊 OLD: Using F32 audio buffer (platform preference)");
                "F32 Buffer"
            }
        };

        // NEW FIX: Use actual stream format
        let new_buffer_type = match use_format {
            cpal::SampleFormat::I16 => {
                println!("   🔊 NEW: Using I16 audio buffer (matches stream format)");
                "I16 Buffer"
            }
            cpal::SampleFormat::F32 => {
                println!("   🔊 NEW: Using F32 audio buffer (matches stream format)");
                "F32 Buffer"
            }
            _ => {
                println!("   🔊 NEW: Using F32 audio buffer (default for unsupported)");
                "F32 Buffer"
            }
        };

        println!(
            "   📊 Result: Stream format = {:?}, Old buffer = {}, New buffer = {}",
            use_format, old_buffer_type, new_buffer_type
        );

        // Check for mismatches
        let old_mismatch = match (use_format, old_buffer_type) {
            (cpal::SampleFormat::F32, "I16 Buffer") => true,
            (cpal::SampleFormat::I16, "F32 Buffer") => true,
            _ => false,
        };

        let new_mismatch = match (use_format, new_buffer_type) {
            (cpal::SampleFormat::F32, "I16 Buffer") => true,
            (cpal::SampleFormat::I16, "F32 Buffer") => true,
            _ => false,
        };

        if old_mismatch {
            println!(
                "   ❌ OLD BUG: Stream uses {:?} but buffer was {}",
                use_format, old_buffer_type
            );
        } else {
            println!("   ✅ OLD: Format consistency was OK");
        }

        if new_mismatch {
            println!(
                "   ❌ NEW BUG: Stream uses {:?} but buffer is {}",
                use_format, new_buffer_type
            );
        } else {
            println!("   ✅ NEW FIX: Format consistency is OK!");
        }
    }

    Ok(())
}

fn test_format_conversion() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n🎯 Test 3: Format Conversion Test");
    println!("---------------------------------");

    // Test data: simple sine wave
    let test_samples_f32: Vec<f32> = (0..8).map(|i| (i as f32 * 0.1).sin()).collect();

    println!("🔢 Original F32 samples: {:?}", test_samples_f32);

    // Convert F32 to I16 (what should happen when hardware gives F32 but buffer wants I16)
    let converted_i16: Vec<i16> = test_samples_f32
        .iter()
        .map(|&sample| {
            let clamped = sample.clamp(-1.0, 1.0);
            (clamped * 32767.0) as i16
        })
        .collect();

    println!("🔢 Converted to I16: {:?}", converted_i16);

    // Convert back to F32 (what should happen when reading from I16 buffer)
    let converted_back_f32: Vec<f32> = converted_i16
        .iter()
        .map(|&sample| sample as f32 / 32767.0)
        .collect();

    println!("🔢 Converted back to F32: {:?}", converted_back_f32);

    // Check for significant loss
    let max_error = test_samples_f32
        .iter()
        .zip(converted_back_f32.iter())
        .map(|(orig, conv)| (orig - conv).abs())
        .fold(0.0f32, |acc, x| acc.max(x));

    println!("📊 Maximum conversion error: {:.6}", max_error);

    if max_error > 0.001 {
        println!("⚠️  High conversion error detected!");
    } else {
        println!("✅ Conversion accuracy: Good");
    }

    // Test what happens with wrong conversion (simulating the bug)
    println!("\n🔍 Simulating format mismatch bug:");

    // Simulate: F32 data interpreted as I16 bytes (the bug scenario)
    let f32_bytes: Vec<u8> = test_samples_f32
        .iter()
        .flat_map(|&f| f.to_le_bytes())
        .collect();

    println!("🔢 F32 as bytes: {} bytes", f32_bytes.len());

    // Try to interpret F32 bytes as I16 samples (this is the bug!)
    let misinterpreted_i16: Vec<i16> = f32_bytes
        .chunks_exact(2)
        .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
        .collect();

    println!(
        "🔢 F32 bytes misinterpreted as I16: {:?}",
        misinterpreted_i16
    );
    println!("❌ This shows data corruption when formats don't match!");

    Ok(())
}
