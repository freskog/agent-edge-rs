use audio::protocol::{ConsumerConnection, ConsumerMessage, ProducerConnection, ProducerMessage};
use std::io::{self, Write};
use std::net::TcpStream;
use std::thread;
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🧪 Binary Audio Service Test Client");
    println!("This will test both consumer and producer interfaces");

    // Test Consumer Interface
    println!("\n🎯 Testing Consumer Interface (port 8080)...");
    test_consumer()?;

    // Test Producer Interface
    println!("\n🔊 Testing Producer Interface (port 8081)...");
    test_producer()?;

    println!("\n✅ All tests passed!");
    Ok(())
}

fn test_consumer() -> Result<(), Box<dyn std::error::Error>> {
    println!("  📡 Connecting to consumer server...");

    let stream = TcpStream::connect("127.0.0.1:8080")?;
    let mut connection = ConsumerConnection::new(stream);

    println!("  ✅ Consumer connected successfully!");

    // Read a few audio chunks to verify streaming and debug VAD
    println!("  🎵 Reading audio chunks for 10 seconds...");
    println!("  Legend: S=Speech detected, .=Silence, chunks shown every 100ms");
    let start_time = std::time::Instant::now();
    let mut chunk_count = 0;
    let mut speech_count = 0;
    let mut silence_count = 0;

    while start_time.elapsed() < Duration::from_secs(10) {
        // Set a short timeout for reads
        match connection.read_message() {
            Ok(ConsumerMessage::Audio {
                data,
                speech_detected,
                timestamp,
            }) => {
                chunk_count += 1;
                if speech_detected {
                    print!("S"); // Speech detected
                    speech_count += 1;
                } else {
                    print!("."); // Silence
                    silence_count += 1;
                }
                io::stdout().flush().unwrap();

                // Print stats every 50 chunks (roughly every 3.2 seconds at 16kHz)
                if chunk_count % 50 == 0 {
                    println!(
                        "\n  📊 After {} chunks: {} speech, {} silence ({:.1}% speech)",
                        chunk_count,
                        speech_count,
                        silence_count,
                        (speech_count as f32 / chunk_count as f32) * 100.0
                    );
                    println!(
                        "  🔍 Latest chunk: {} bytes, speech={}, timestamp={}",
                        data.len(),
                        speech_detected,
                        timestamp
                    );
                }

                if chunk_count >= 200 {
                    // Don't flood the output
                    break;
                }
            }
            Ok(ConsumerMessage::WakewordDetected { model, .. }) => {
                println!("\n  🎯 Wake word detected: {}", model);
            }
            Ok(other) => {
                println!("\n  📨 Unexpected message: {:?}", other);
            }
            Err(_) => {
                // Timeout or connection issue, that's expected for this test
                thread::sleep(Duration::from_millis(50));
            }
        }
    }

    println!(
        "\n  ✅ Consumer test completed! Received {} audio chunks",
        chunk_count
    );
    Ok(())
}

fn test_producer() -> Result<(), Box<dyn std::error::Error>> {
    println!("  📡 Connecting to producer server...");

    let stream = TcpStream::connect("127.0.0.1:8081")?;
    let mut connection = ProducerConnection::new(stream);

    println!("  ✅ Producer connected successfully!");

    // Send some test audio data
    println!("  🎵 Sending test audio chunks...");

    // Generate some test audio data (simple sine wave at 44.1kHz)
    let sample_rate = 44100;
    let duration_secs = 1.0;
    let frequency = 440.0; // A4 note
    let samples_count = (sample_rate as f32 * duration_secs) as usize;

    // Generate sine wave and convert to s16le bytes
    let mut audio_data = Vec::new();
    for i in 0..samples_count {
        let t = i as f32 / sample_rate as f32;
        let sample = (2.0 * std::f32::consts::PI * frequency * t).sin();
        let sample_i16 = (sample * i16::MAX as f32) as i16;
        audio_data.extend_from_slice(&sample_i16.to_le_bytes());
    }

    // Send audio in chunks
    let chunk_size = 4096; // 2048 samples worth of data
    let mut sent_chunks = 0;

    for chunk in audio_data.chunks(chunk_size) {
        let play_msg = ProducerMessage::Play {
            data: chunk.to_vec(),
        };
        connection.write_message(&play_msg)?;
        sent_chunks += 1;

        // Small delay between chunks
        thread::sleep(Duration::from_millis(50));

        print!(".");
        io::stdout().flush().unwrap();

        if sent_chunks >= 10 {
            // Don't send too much test data
            break;
        }
    }

    println!("\n  🎵 Sent {} audio chunks", sent_chunks);

    // Test stop command
    println!("  🛑 Testing Stop command...");
    let stop_msg = ProducerMessage::Stop {
        timestamp: ProducerMessage::current_timestamp(),
    };
    connection.write_message(&stop_msg)?;

    println!("  ✅ Producer test completed!");
    Ok(())
}
