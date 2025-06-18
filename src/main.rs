use agent_edge_rs::audio::{AudioCapture, AudioCaptureConfig};
#[cfg(all(target_os = "linux", feature = "pulseaudio"))]
use agent_edge_rs::audio::{PulseAudioCapture, PulseAudioCaptureConfig};
use anyhow::Result;
use clap::Parser;
use log::{error, info};

#[derive(Parser)]
#[command(name = "agent-edge")]
#[command(about = "Wakeword-only edge client for low-powered devices")]
struct Args {
    /// Enable verbose logging
    #[arg(short, long)]
    verbose: bool,

    /// Audio device name (optional, will use default if not specified)
    #[arg(short, long)]
    device: Option<String>,

    /// List available audio input devices and exit
    #[arg(long)]
    list_devices: bool,

    /// Duration to capture audio in seconds (for testing)
    #[arg(long, default_value = "5")]
    duration: u64,

    /// Development mode: auto-detect best available audio config
    #[arg(long)]
    dev_mode: bool,

    /// Use PulseAudio directly (Linux only) for better latency control
    #[arg(long)]
    use_pulseaudio: bool,

    /// Target latency in milliseconds - communicated to PulseAudio server (only with --use-pulseaudio)
    #[arg(long, default_value = "50")]
    latency_ms: u32,
}

fn main() -> Result<()> {
    let args = Args::parse();

    // Initialize logging
    let log_level = if args.verbose { "debug" } else { "info" };
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(log_level)).init();

    info!("Starting agent-edge wakeword client");
    info!(
        "Target platform: {} on {}",
        std::env::consts::ARCH,
        std::env::consts::OS
    );

    // Check audio system availability and PulseAudio option
    if args.use_pulseaudio {
        #[cfg(all(target_os = "linux", feature = "pulseaudio"))]
        {
            info!(
                "Audio system: Direct PulseAudio ({}ms target latency communicated to server)",
                args.latency_ms
            );
            return run_with_pulseaudio(args);
        }
        #[cfg(not(all(target_os = "linux", feature = "pulseaudio")))]
        {
            error!("PulseAudio support is only available on Linux with pulseaudio feature enabled");
            error!(
                "On your platform ({}), please use regular audio capture without --use-pulseaudio",
                std::env::consts::OS
            );
            return Err(anyhow::anyhow!("PulseAudio feature not available"));
        }
    } else {
        // Show audio system info
        #[cfg(target_os = "linux")]
        info!("Audio system: cpal -> ALSA -> PulseAudio");
        #[cfg(target_os = "macos")]
        info!("Audio system: Core Audio (via cpal)");
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        info!("Audio system: Unknown platform - using cpal defaults");

        run_with_cpal(args)
    }
}

#[cfg(all(target_os = "linux", feature = "pulseaudio"))]
fn run_with_pulseaudio(args: Args) -> Result<()> {
    info!("Using direct PulseAudio capture");

    // Create PulseAudio configuration
    let pulse_config = if args.dev_mode {
        PulseAudioCaptureConfig {
            sample_rate: 16000,
            channels: 1, // Start with mono for dev compatibility
            device_name: args.device.clone(),
            target_latency_ms: args.latency_ms,
            app_name: "agent-edge".to_string(),
            stream_name: "dev-wakeword-capture".to_string(),
        }
    } else {
        PulseAudioCaptureConfig {
            sample_rate: 16000,
            channels: 6, // ReSpeaker 4-mic array
            device_name: args.device.clone(),
            target_latency_ms: args.latency_ms,
            app_name: "agent-edge".to_string(),
            stream_name: "wakeword-capture".to_string(),
        }
    };

    info!("PulseAudio configuration: {:?}", pulse_config);

    // Create and start PulseAudio capture
    let mut pulse_capture = PulseAudioCapture::new(pulse_config)?;
    pulse_capture.start()?;

    info!("Starting PulseAudio capture for {} seconds", args.duration);

    let start_time = std::time::Instant::now();
    let mut sample_count = 0usize;
    let mut max_amplitude = 0.0f32;
    let mut min_amplitude = 0.0f32;

    while start_time.elapsed().as_secs() < args.duration {
        match pulse_capture.try_get_audio_buffer() {
            Ok(Some(buffer)) => {
                sample_count += buffer.len();

                // Calculate some basic statistics
                for &sample in &buffer {
                    max_amplitude = max_amplitude.max(sample);
                    min_amplitude = min_amplitude.min(sample);
                }

                // Log periodic statistics
                if sample_count % 16000 == 0 {
                    // Every ~1 second at 16kHz
                    info!(
                        "Captured {} samples, amplitude range: [{:.6}, {:.6}]",
                        sample_count, min_amplitude, max_amplitude
                    );
                }
            }
            Ok(None) => {
                // No data available, sleep a bit
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            Err(e) => {
                error!("Failed to get audio buffer: {}", e);
                break;
            }
        }
    }

    pulse_capture.stop()?;

    info!("PulseAudio capture completed");
    info!("Total samples captured: {}", sample_count);
    info!(
        "Final amplitude range: [{:.6}, {:.6}]",
        min_amplitude, max_amplitude
    );

    let duration_secs = start_time.elapsed().as_secs_f64();
    if duration_secs > 0.0 {
        let sample_rate = sample_count as f64 / duration_secs;
        info!("Average sample rate: {:.1} Hz", sample_rate);
    }

    Ok(())
}

fn run_with_cpal(args: Args) -> Result<()> {
    info!("Using cpal audio interface");

    // Create audio capture configuration
    let config = if args.dev_mode {
        info!("Development mode: detecting best available audio config");
        AudioCaptureConfig {
            sample_rate: 16000,
            channels: 1, // Start with mono for dev compatibility
            device_name: args.device.clone(),
            buffer_size: 1024,
            target_latency_ms: args.latency_ms, // 50ms target latency for PulseAudio
        }
    } else {
        // Production mode: ReSpeaker 4-mic array
        let mut config = AudioCaptureConfig::default();
        config.device_name = args.device.clone();
        config.target_latency_ms = args.latency_ms;
        config
    };

    if let Some(device) = &args.device {
        info!("Using audio device: {}", device);
    } else {
        info!("Using default audio device");
    }

    // Initialize audio capture
    let mut audio_capture = AudioCapture::new(config)?;

    // Handle list-devices option
    if args.list_devices {
        info!("Available audio input devices:");
        match audio_capture.list_input_devices() {
            Ok(devices) => {
                for (i, device) in devices.iter().enumerate() {
                    println!("  {}: {}", i, device);
                }
                return Ok(());
            }
            Err(e) => {
                error!("Failed to list audio devices: {}", e);
                return Err(e.into());
            }
        }
    }

    // Start audio capture
    info!("Starting audio capture for {} seconds", args.duration);
    audio_capture.start()?;

    // Capture audio for the specified duration
    let start_time = std::time::Instant::now();
    let mut sample_count = 0;
    let mut total_samples = 0;

    while start_time.elapsed().as_secs() < args.duration {
        match audio_capture.try_get_audio_buffer() {
            Ok(Some(buffer)) => {
                sample_count += 1;
                total_samples += buffer.len();

                if sample_count % 100 == 0 {
                    info!(
                        "Captured {} buffers, {} total samples",
                        sample_count, total_samples
                    );

                    // Show some audio statistics
                    if !buffer.is_empty() {
                        let avg = buffer.iter().sum::<f32>() / buffer.len() as f32;
                        let max = buffer.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
                        info!(
                            "  Latest buffer: {} samples, avg: {:.4}, max: {:.4}",
                            buffer.len(),
                            avg,
                            max
                        );
                    }
                }
            }
            Ok(None) => {
                // No audio data available yet
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            Err(e) => {
                error!("Audio capture error: {}", e);
                break;
            }
        }
    }

    info!(
        "Captured {} audio buffers with {} total samples",
        sample_count, total_samples
    );

    // Stop audio capture
    audio_capture.stop()?;
    info!("Audio capture test completed successfully");

    // TODO: Load TensorFlow Lite models
    // TODO: Start wakeword detection loop

    Ok(())
}
