use clap::{Arg, Command};
use env_logger::Env;
use log::{error, info};
use std::time::Duration;
use tokio::signal;
use tokio::time::sleep;

use agent_edge_rs::audio::{AudioBuffer, PulseAudioCapture, PulseAudioCaptureConfig};
use agent_edge_rs::detection::{DetectionPipeline, PipelineConfig};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    // CLI argument parsing
    let matches = Command::new("agent-edge")
        .version("0.1.0")
        .about("Wakeword detection for Raspberry Pi edge devices")
        .arg(
            Arg::new("verbose")
                .long("verbose")
                .short('v')
                .action(clap::ArgAction::SetTrue)
                .help("Enable verbose debug logging"),
        )
        .arg(
            Arg::new("device")
                .long("device")
                .value_name("NAME")
                .help("Use specific PulseAudio device name"),
        )
        .arg(
            Arg::new("threshold")
                .long("threshold")
                .value_name("FLOAT")
                .help("Wakeword confidence threshold (0.0-1.0)")
                .value_parser(clap::value_parser!(f32))
                .default_value("0.8"),
        )
        .arg(
            Arg::new("latency")
                .long("latency")
                .value_name("MS")
                .help("Target audio latency in milliseconds")
                .value_parser(clap::value_parser!(u32))
                .default_value("50"),
        )
        .get_matches();

    // Set log level based on verbose flag
    if matches.get_flag("verbose") {
        log::set_max_level(log::LevelFilter::Debug);
    }

    info!("🚀 Starting agent-edge wakeword detection");
    info!(
        "   Platform: {} on {}",
        std::env::consts::ARCH,
        std::env::consts::OS
    );

    // Configure PulseAudio capture
    let latency_ms = matches.get_one::<u32>("latency").copied().unwrap_or(50);
    let audio_config = PulseAudioCaptureConfig {
        sample_rate: 16000,
        channels: 6, // ReSpeaker 4-mic array (6 channels)
        device_name: matches.get_one::<String>("device").cloned(),
        target_latency_ms: latency_ms,
        app_name: "agent-edge".to_string(),
        stream_name: "wakeword-capture".to_string(),
    };

    // Configure detection pipeline
    let mut pipeline_config = PipelineConfig::default();
    let threshold = matches.get_one::<f32>("threshold").copied().unwrap_or(0.8);
    pipeline_config.wakeword_config.confidence_threshold = threshold;
    pipeline_config.debug_mode = matches.get_flag("verbose");

    // Create pipeline
    info!("Initializing wakeword detection pipeline...");
    let mut pipeline = DetectionPipeline::new(pipeline_config)?;

    // Create PulseAudio capture
    info!("Setting up PulseAudio capture...");
    let mut audio_capture = PulseAudioCapture::new(audio_config)?;
    audio_capture.start()?;

    info!("🎤 Starting audio capture...");
    info!(
        "   Chunk size: {} samples ({}ms)",
        pipeline.chunk_size_samples(),
        pipeline.chunk_duration_ms()
    );
    info!("   Detection threshold: {:.1}", pipeline.get_threshold());
    info!("   Target latency: {}ms", latency_ms);
    info!("   Listening for wakeword 'hey mycroft'... (Press Ctrl+C to stop)");

    let start_time = std::time::Instant::now();
    let mut chunk_count = 0;
    let chunk_size = pipeline.chunk_size_samples();

    // Main processing loop - run until Ctrl+C
    loop {
        // Check for Ctrl+C signal
        tokio::select! {
            _ = signal::ctrl_c() => {
                info!("Received Ctrl+C, shutting down...");
                break;
            }

            // Try to get audio data
            result = async {
                match audio_capture.try_get_audio_buffer() {
                    Ok(Some(mut audio_buffer)) => {
                        // Process buffer in chunks of the required size
                        while audio_buffer.len() >= chunk_size {
                            let audio_chunk: AudioBuffer = audio_buffer.drain(..chunk_size).collect();
                            chunk_count += 1;

                            // Process through wakeword pipeline
                            match pipeline.process_chunk(&audio_chunk) {
                                Ok(Some(detection)) => {
                                    if detection.detected {
                                        info!("🎯 WAKEWORD DETECTED!");
                                        info!("   Confidence: {:.3}", detection.confidence);
                                        info!("   Frame: {}", detection.frame_number);

                                        // Reset pipeline after detection
                                        pipeline.reset();

                                        // In production, this is where you'd trigger the next stage
                                        // (ASR, command processing, etc.)
                                    } else if detection.confidence >= threshold {
                                        // Show detection scores above threshold even if not triggered
                                        info!("   Detection: {:.3} (frame {})",
                                              detection.confidence, detection.frame_number);
                                    } else if matches.get_flag("verbose") {
                                        info!("   Frame {}: confidence {:.3}",
                                              detection.frame_number, detection.confidence);
                                    }
                                }
                                Ok(None) => {
                                    // Still accumulating frames for detection
                                    if matches.get_flag("verbose") && chunk_count % 50 == 0 {
                                        let stats = pipeline.stats();
                                        info!("Processed {} chunks, buffer: {}/{}",
                                              stats.chunks_processed,
                                              stats.frames_buffered,
                                              76);
                                    }
                                }
                                Err(e) => {
                                    error!("Detection error: {}", e);
                                    return Err(e.into());
                                }
                            }
                        }
                    }
                    Ok(None) => {
                        // No audio data available, sleep briefly
                        sleep(Duration::from_millis(1)).await;
                    }
                    Err(e) => {
                        error!("Audio capture error: {}", e);
                        return Err(e.into());
                    }
                }
                Ok::<(), anyhow::Error>(())
            } => {
                if let Err(e) = result {
                    return Err(e);
                }
            }
        }
    }

    // Stop audio capture
    audio_capture.stop()?;

    // Show final statistics
    let stats = pipeline.stats();
    info!("📊 Session complete:");
    info!("   Total chunks processed: {}", stats.chunks_processed);
    info!(
        "   Average processing time: {:.2}ms",
        stats.avg_processing_time_ms
    );
    info!(
        "   Total runtime: {:.1}s",
        start_time.elapsed().as_secs_f32()
    );

    Ok(())
}
