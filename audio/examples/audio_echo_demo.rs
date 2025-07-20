#!/usr/bin/env cargo

//! Audio Echo Demo
//!
//! This demo connects to the audio API server via TCP and:
//! 1. Records audio for a specified duration
//! 2. Saves the audio to a PCM file
//! 3. Plays the recorded audio back
//!
//! Usage:
//!   cargo run --example audio_echo_demo -- --duration 5
//!   cargo run --example audio_echo_demo -- --duration 10 --output recording.pcm --server 127.0.0.1:50051

use audio::resampler::SimpleResampler;
use audio_protocol::{AudioChunk, AudioClient};
use clap::Parser;
use log::{error, info, warn};
use std::fs::File;
use std::io::Write;

use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Parser)]
#[command(name = "audio_echo_demo")]
#[command(about = "Record and playback audio via TCP protocol")]
#[command(long_about = "
Audio echo demo that demonstrates the audio_protocol TCP client.

This tool connects to the audio API server, records audio for the specified 
duration, saves it to a PCM file, and then plays it back.

EXAMPLES:
  # Record for 5 seconds with default settings
  audio_echo_demo --duration 5
  
  # Record for 10 seconds and save to custom file
  audio_echo_demo --duration 10 --output my_recording.pcm
  
  # Connect to different server
  audio_echo_demo --duration 5 --server 192.168.1.100:50051
  
  # Just record, don't play back
  audio_echo_demo --duration 5 --no-playback
")]
struct Args {
    /// Duration to record in seconds
    #[arg(short, long)]
    duration: u64,

    /// Output PCM file path (default: auto-generated)
    #[arg(short, long)]
    output: Option<String>,

    /// Audio server address
    #[arg(short, long, default_value = "127.0.0.1:50051")]
    server: String,

    /// Countdown duration before recording starts
    #[arg(short, long, default_value = "3")]
    countdown: u64,

    /// Skip playback, just record and save
    #[arg(long)]
    no_playback: bool,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    let args = Args::parse();

    // Generate output filename if not provided
    let output_path = args.output.unwrap_or_else(|| {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        format!("audio_recording_{}.pcm", timestamp)
    });

    info!("🎤 Audio Echo Demo");
    info!("📡 Server: {}", args.server);
    info!("⏱️  Duration: {}s", args.duration);
    info!("💾 Output: {}", output_path);

    if args.no_playback {
        info!("🔇 Playback disabled");
    }

    // Connect to the audio server
    let mut client = AudioClient::connect(&args.server)?;

    // Start recording
    let recorded_chunks = record_audio(&mut client, args.duration, args.countdown)?;

    // Save to file
    let bytes_saved = save_to_file(&recorded_chunks, &output_path)?;
    info!("💾 Saved {} bytes to {}", bytes_saved, output_path);

    // Write metadata file
    write_metadata_file(&output_path, bytes_saved, recorded_chunks.len())?;

    // Play back if requested
    if !args.no_playback && !recorded_chunks.is_empty() {
        playback_audio(&mut client, &recorded_chunks, &output_path)?;
    }

    info!("✅ Demo completed successfully!");
    Ok(())
}

/// Record audio for the specified duration
fn record_audio(
    client: &mut AudioClient,
    duration_secs: u64,
    countdown_secs: u64,
) -> Result<Vec<AudioChunk>, Box<dyn std::error::Error>> {
    // Countdown
    if countdown_secs > 0 {
        info!("🕐 Recording starts in:");
        for i in (1..=countdown_secs).rev() {
            println!("   {}...", i);
            thread::sleep(Duration::from_secs(1));
        }
    }

    info!("🔴 RECORDING... ({}s)", duration_secs);

    // Subscribe to audio capture
    client.subscribe_audio()?;

    let mut chunks = Vec::new();
    let start_time = Instant::now();
    let duration = Duration::from_secs(duration_secs);
    let mut total_bytes = 0;
    let mut chunk_count = 0;

    // Record until duration expires
    while start_time.elapsed() < duration {
        match client.read_audio_chunk() {
            Ok(Some(chunk)) => {
                total_bytes += chunk.size_bytes();
                chunk_count += 1;

                if chunk_count % 10 == 0 {
                    let elapsed = start_time.elapsed().as_secs();
                    let remaining = duration_secs.saturating_sub(elapsed);
                    info!(
                        "📥 Chunk {}: {} bytes ({}s remaining)",
                        chunk_count,
                        chunk.size_bytes(),
                        remaining
                    );
                }

                chunks.push(chunk);
            }
            Ok(None) => {
                warn!("⚠️  Received None chunk (server error)");
                break;
            }
            Err(e) => {
                error!("❌ Error reading chunk: {}", e);
                break;
            }
        }
    }

    info!(
        "✅ Recording complete: {} chunks, {} bytes total",
        chunks.len(),
        total_bytes
    );

    Ok(chunks)
}

/// Save audio chunks to PCM file
fn save_to_file(chunks: &[AudioChunk], path: &str) -> Result<usize, Box<dyn std::error::Error>> {
    info!("💾 Saving to {}...", path);

    let mut file = File::create(path)?;
    let mut total_bytes = 0;

    for (i, chunk) in chunks.iter().enumerate() {
        file.write_all(&chunk.data)?;
        total_bytes += chunk.data.len();

        if (i + 1) % 50 == 0 {
            info!("💾 Saved chunk {}/{}", i + 1, chunks.len());
        }
    }

    file.flush()?;

    Ok(total_bytes)
}

/// Write metadata file with recording information
fn write_metadata_file(
    pcm_path: &str,
    bytes_saved: usize,
    chunk_count: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let metadata_path = format!("{}.txt", pcm_path);
    let mut file = File::create(&metadata_path)?;

    let samples = bytes_saved / 2; // s16le = 2 bytes per sample
    let duration_secs = samples as f64 / 16000.0; // 16kHz sample rate

    writeln!(file, "Audio Recording Metadata")?;
    writeln!(file, "========================")?;
    writeln!(file, "Format: s16le (16-bit signed little-endian)")?;
    writeln!(file, "Sample Rate: 16kHz")?;
    writeln!(file, "Channels: 1 (mono)")?;
    writeln!(file, "Bytes: {}", bytes_saved)?;
    writeln!(file, "Samples: {}", samples)?;
    writeln!(file, "Chunks: {}", chunk_count)?;
    writeln!(file, "Duration: {:.2}s", duration_secs)?;
    writeln!(file, "")?;
    writeln!(file, "To play with ffplay:")?;
    writeln!(file, "ffplay -f s16le -ar 16000 -ac 1 {}", pcm_path)?;

    info!("📝 Metadata saved to {}", metadata_path);
    Ok(())
}

/// Play back the recorded audio
fn playback_audio(
    client: &mut AudioClient,
    chunks: &[AudioChunk],
    output_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if chunks.is_empty() {
        warn!("⚠️  No audio to play back");
        return Ok(());
    }

    info!("🔊 Preparing audio for playback...");

    // Step 1: Unsubscribe from audio capture to avoid receiving AudioChunk messages during playback
    info!("📤 Unsubscribing from audio capture...");
    match client.unsubscribe_audio() {
        Ok(result) => {
            if result.success {
                info!("✅ Unsubscribed successfully: {}", result.message);
            } else {
                warn!("⚠️  Unsubscribe warning: {}", result.message);
            }
        }
        Err(e) => {
            warn!("⚠️  Failed to unsubscribe: {}", e);
        }
    }

    // Step 2: Resample audio from 16kHz to 44.1kHz
    info!("🔄 Resampling audio from 16kHz to 44.1kHz...");
    let mut resampler = SimpleResampler::new()?;
    let mut resampled_chunks = Vec::new();
    let mut total_input_bytes = 0;
    let mut total_output_bytes = 0;

    for (i, chunk) in chunks.iter().enumerate() {
        match resampler.resample_s16le(&chunk.data) {
            Ok(resampled_data) => {
                total_input_bytes += chunk.data.len();
                total_output_bytes += resampled_data.len();
                resampled_chunks.push(resampled_data);

                if (i + 1) % 50 == 0 {
                    info!("🔄 Resampled chunk {}/{}", i + 1, chunks.len());
                }
            }
            Err(e) => {
                error!("❌ Failed to resample chunk {}: {}", i + 1, e);
                return Err(e);
            }
        }
    }

    // Flush any remaining samples from the resampler
    match resampler.flush() {
        Ok(final_data) => {
            if !final_data.is_empty() {
                let final_bytes = final_data.len();
                total_output_bytes += final_bytes;
                resampled_chunks.push(final_data);
                info!("🔄 Flushed final {} bytes from resampler", final_bytes);
            }
        }
        Err(e) => {
            error!("❌ Failed to flush resampler: {}", e);
            return Err(e);
        }
    }

    info!(
        "✅ Resampling complete: {} → {} bytes ({:.1}x)",
        total_input_bytes,
        total_output_bytes,
        total_output_bytes as f64 / total_input_bytes as f64
    );

    // Save resampled audio to file
    let resampled_path = output_path.replace(".pcm", "_resampled_44khz.pcm");
    info!("💾 Saving resampled audio to {}...", resampled_path);
    let mut resampled_file = File::create(&resampled_path)?;
    let mut resampled_bytes_saved = 0;

    for chunk_data in &resampled_chunks {
        resampled_file.write_all(chunk_data)?;
        resampled_bytes_saved += chunk_data.len();
    }
    resampled_file.flush()?;

    info!(
        "💾 Saved {} bytes of resampled audio to {}",
        resampled_bytes_saved, resampled_path
    );

    // Write metadata for resampled file
    let resampled_metadata_path = format!("{}.txt", resampled_path);
    let mut resampled_meta_file = File::create(&resampled_metadata_path)?;
    let resampled_samples = resampled_bytes_saved / 2; // s16le = 2 bytes per sample
    let resampled_duration_secs = resampled_samples as f64 / 44100.0; // 44.1kHz sample rate

    writeln!(resampled_meta_file, "Resampled Audio Recording Metadata")?;
    writeln!(resampled_meta_file, "==================================")?;
    writeln!(
        resampled_meta_file,
        "Format: s16le (16-bit signed little-endian)"
    )?;
    writeln!(resampled_meta_file, "Sample Rate: 44.1kHz")?;
    writeln!(resampled_meta_file, "Channels: 1 (mono)")?;
    writeln!(resampled_meta_file, "Bytes: {}", resampled_bytes_saved)?;
    writeln!(resampled_meta_file, "Samples: {}", resampled_samples)?;
    writeln!(
        resampled_meta_file,
        "Duration: {:.2}s",
        resampled_duration_secs
    )?;
    writeln!(resampled_meta_file, "")?;
    writeln!(resampled_meta_file, "To play with ffplay:")?;
    writeln!(
        resampled_meta_file,
        "ffplay -f s16le -ar 44100 -ac 1 {}",
        resampled_path
    )?;

    info!("📝 Resampled metadata saved to {}", resampled_metadata_path);

    // Step 3: Play back the resampled audio
    info!(
        "🔊 Playing back {} resampled chunks...",
        resampled_chunks.len()
    );

    let stream_id = "echo-demo";
    let mut successful_chunks = 0;
    let mut failed_chunks = 0;

    for (i, chunk_data) in resampled_chunks.iter().enumerate() {
        match client.play_audio_chunk(stream_id, chunk_data.clone()) {
            Ok(result) => {
                if result.success {
                    successful_chunks += 1;
                    if (i + 1) % 20 == 0 {
                        info!("📤 Sent chunk {}/{}", i + 1, resampled_chunks.len());
                    }
                } else {
                    error!("❌ Play failed for chunk {}: {}", i + 1, result.message);
                    failed_chunks += 1;
                }
            }
            Err(e) => {
                error!("❌ Error sending chunk {}: {}", i + 1, e);
                failed_chunks += 1;
            }
        }

        // No artificial delay - let the audio sink provide natural backpressure
    }

    // Signal end of stream
    match client.end_stream(stream_id) {
        Ok(result) => {
            if result.success {
                info!(
                    "✅ Playback complete: {} successful, {} failed",
                    successful_chunks, failed_chunks
                );
                info!("🎵 Server played {} chunks total", result.chunks_played);
            } else {
                error!("❌ End stream failed: {}", result.message);
            }
        }
        Err(e) => {
            error!("❌ Error ending stream: {}", e);
        }
    }

    Ok(())
}
