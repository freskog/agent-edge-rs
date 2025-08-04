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

    // Send Subscribe message
    println!("  📨 Sending Subscribe message...");
    let subscribe_msg = ConsumerMessage::Subscribe {
        id: "test-client".to_string(),
    };
    connection.write_message(&subscribe_msg)?;

    // Read Connected response
    println!("  📥 Waiting for Connected response...");
    match connection.read_message()? {
        ConsumerMessage::Connected => {
            println!("  ✅ Consumer connected successfully!");
        }
        ConsumerMessage::Error { message } => {
            println!("  ❌ Consumer connection error: {}", message);
            return Err(format!("Consumer error: {}", message).into());
        }
        other => {
            println!("  ❌ Unexpected message: {:?}", other);
            return Err("Unexpected response".into());
        }
    }

    // Read a few audio chunks to verify streaming
    println!("  🎵 Reading audio chunks for 2 seconds...");
    let start_time = std::time::Instant::now();
    let mut chunk_count = 0;

    while start_time.elapsed() < Duration::from_secs(2) {
        // Set a short timeout for reads
        match connection.read_message() {
            Ok(ConsumerMessage::Audio { data, speech_detected }) => {
                chunk_count += 1;
                if speech_detected {
                    print!("S"); // Speech detected
                } else {
                    print!("."); // Silence
                }
                io::stdout().flush().unwrap();

                if chunk_count >= 50 {
                    // Don't flood the output
                    break;
                }
            }
            Ok(ConsumerMessage::WakewordDetected { model }) => {
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

    // Read Connected response (producer sends this immediately)
    println!("  📥 Waiting for Connected response...");
    match connection.read_message()? {
        ProducerMessage::Connected => {
            println!("  ✅ Producer connected successfully!");
        }
        ProducerMessage::Error { message } => {
            println!("  ❌ Producer connection error: {}", message);
            return Err(format!("Producer error: {}", message).into());
        }
        other => {
            println!("  ❌ Unexpected message: {:?}", other);
            return Err("Unexpected response".into());
        }
    }

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
    let stop_msg = ProducerMessage::Stop;
    connection.write_message(&stop_msg)?;

    println!("  ✅ Producer test completed!");
    Ok(())
}
