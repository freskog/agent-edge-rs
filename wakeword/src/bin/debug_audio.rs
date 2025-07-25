use anyhow::Result;
use audio_protocol::client::AudioClient;
use clap::Parser;
use std::fs::File;
use std::io::Write;

#[derive(Parser)]
#[command(name = "debug_audio")]
#[command(about = "Capture and analyze audio from different sources")]
struct Args {
    /// Audio server address
    #[arg(short, long, default_value = "127.0.0.1:50051")]
    server: String,

    /// Number of chunks to capture
    #[arg(short, long, default_value = "50")]
    chunks: usize,

    /// Output file prefix
    #[arg(short, long, default_value = "debug_audio")]
    output: String,
}

fn main() -> Result<()> {
    env_logger::init();
    let args = Args::parse();

    println!("🔍 Connecting to audio server: {}", args.server);
    let mut client = AudioClient::connect(&args.server)?;

    println!("🎤 Subscribing to audio stream...");
    client.subscribe_audio()?;

    let mut chunks_data = Vec::new();
    let mut chunk_count = 0;

    println!("📡 Capturing {} chunks...", args.chunks);

    while chunk_count < args.chunks {
        if let Some(chunk) = client.read_audio_chunk()? {
            chunks_data.push(chunk.data.clone());
            chunk_count += 1;

            if chunk_count % 10 == 0 {
                println!("📥 Captured {}/{} chunks", chunk_count, args.chunks);
            }

            // Analyze first chunk
            if chunk_count == 1 {
                analyze_chunk(&chunk.data, chunk_count);
            }
        }
    }

    // Save raw audio data
    let output_file = format!("{}.raw", args.output);
    let mut file = File::create(&output_file)?;

    for chunk_data in &chunks_data {
        file.write_all(chunk_data)?;
    }

    println!("💾 Saved {} chunks to {}", chunks_data.len(), output_file);
    println!(
        "📊 Total audio: {:.2}s",
        chunks_data.len() as f32 * 1280.0 / 16000.0
    );

    // Analyze overall statistics
    analyze_overall(&chunks_data);

    Ok(())
}

fn analyze_chunk(data: &[u8], chunk_num: usize) {
    // Convert to i16 samples
    let samples: Vec<i16> = data
        .chunks_exact(2)
        .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
        .collect();

    if samples.is_empty() {
        return;
    }

    // Calculate statistics
    let mean = samples.iter().map(|&x| x as f64).sum::<f64>() / samples.len() as f64;
    let min = samples.iter().min().unwrap_or(&0);
    let max = samples.iter().max().unwrap_or(&0);
    let rms =
        (samples.iter().map(|&x| (x as f64).powi(2)).sum::<f64>() / samples.len() as f64).sqrt();

    println!("🔍 Chunk {}: {} samples", chunk_num, samples.len());
    println!(
        "   Mean: {:.2}, Min: {}, Max: {}, RMS: {:.2}",
        mean, min, max, rms
    );
    println!("   First 8 samples: {:?}", &samples[..samples.len().min(8)]);
}

fn analyze_overall(chunks_data: &[Vec<u8>]) {
    let total_bytes: usize = chunks_data.iter().map(|c| c.len()).sum();
    let total_samples = total_bytes / 2;

    // Flatten all samples
    let all_samples: Vec<i16> = chunks_data
        .iter()
        .flat_map(|chunk| {
            chunk
                .chunks_exact(2)
                .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
        })
        .collect();

    if all_samples.is_empty() {
        return;
    }

    let mean = all_samples.iter().map(|&x| x as f64).sum::<f64>() / all_samples.len() as f64;
    let min = all_samples.iter().min().unwrap_or(&0);
    let max = all_samples.iter().max().unwrap_or(&0);
    let rms = (all_samples.iter().map(|&x| (x as f64).powi(2)).sum::<f64>()
        / all_samples.len() as f64)
        .sqrt();

    // Count silence (near-zero samples)
    let silence_count = all_samples.iter().filter(|&&x| x.abs() < 100).count();
    let silence_ratio = silence_count as f64 / all_samples.len() as f64;

    println!("\n📊 Overall Statistics:");
    println!("   Total samples: {}", total_samples);
    println!(
        "   Mean: {:.2}, Min: {}, Max: {}, RMS: {:.2}",
        mean, min, max, rms
    );
    println!("   Silence ratio: {:.1}%", silence_ratio * 100.0);
    println!("   Duration: {:.2}s", total_samples as f32 / 16000.0);
}
