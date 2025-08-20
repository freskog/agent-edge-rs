use std::fs::File;
use std::io::Read;
use std::net::TcpStream;
use std::thread;
use std::time::Duration;

use audio::protocol::{ProducerConnection, ProducerMessage};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 || args.len() > 3 {
        eprintln!("Usage: {} <audio_file.raw> [server_address:port]", args[0]);
        eprintln!();
        eprintln!("Audio file should be mono 48kHz s16le format.");
        eprintln!("Server address defaults to 127.0.0.1:8081");
        eprintln!("To convert from other formats:");
        eprintln!("  ffmpeg -i input.wav -f s16le -ar 48000 -ac 1 output.raw");
        std::process::exit(1);
    }

    let audio_file = &args[1];
    let server_addr = if args.len() == 3 {
        &args[2]
    } else {
        "127.0.0.1:8081"
    };

    // Read the raw audio file
    println!("📁 Reading audio file: {}", audio_file);
    let mut file = File::open(audio_file)?;
    let mut audio_data = Vec::new();
    file.read_to_end(&mut audio_data)?;

    if audio_data.len() % 2 != 0 {
        return Err("Audio file size is not aligned to 16-bit samples".into());
    }

    let sample_count = audio_data.len() / 2;
    let duration_seconds = sample_count as f64 / 48000.0;
    println!(
        "🎵 Audio file: {} samples ({:.2}s at 48kHz)",
        sample_count, duration_seconds
    );

    // Connect to audio server
    println!("🔌 Connecting to audio server at {}...", server_addr);
    let stream = TcpStream::connect(server_addr)?;
    let mut connection = ProducerConnection::new(stream);

    println!("✅ Connected to audio server");

    // Send audio in chunks (simulate streaming)
    const CHUNK_SIZE: usize = 1024 * 2; // 1024 samples = 2048 bytes
    let mut total_sent = 0;

    for chunk in audio_data.chunks(CHUNK_SIZE) {
        let play_msg = ProducerMessage::Play {
            data: chunk.to_vec(),
        };

        connection.write_message(&play_msg)?;
        total_sent += chunk.len();

        let progress = (total_sent as f64 / audio_data.len() as f64) * 100.0;
        println!(
            "📤 Sent {}/{} bytes ({:.1}%)",
            total_sent,
            audio_data.len(),
            progress
        );

        // Small delay to simulate real-time streaming
        thread::sleep(Duration::from_millis(20));
    }

    println!("🎉 Audio file sent successfully!");

    // Send end-of-stream signal
    println!("🏁 Sending end-of-stream signal...");
    let end_msg = ProducerMessage::EndOfStream {
        timestamp: ProducerMessage::current_timestamp(),
    };
    connection.write_message(&end_msg)?;

    // Wait for playback completion
    println!("⏳ Waiting for audio playback to complete...");
    match connection.read_message() {
        Ok(ProducerMessage::PlaybackComplete { timestamp }) => {
            println!("🎉 Audio playback completed at timestamp: {}", timestamp);
        }
        Ok(ProducerMessage::Error { message }) => {
            eprintln!("❌ Server error: {}", message);
            return Err(format!("Server error: {}", message).into());
        }
        Ok(msg) => {
            eprintln!("❌ Unexpected message from server: {:?}", msg);
            return Err("Unexpected server response".into());
        }
        Err(e) => {
            eprintln!("❌ Failed to read server response: {}", e);
            return Err(e.into());
        }
    }

    println!("✅ Test completed successfully");

    Ok(())
}
