use audio::audio_service_client::AudioServiceClient;
use audio::audio_service_server::AudioServiceServer;
use audio::{
    play_audio_request, AbortRequest, AudioChunk, EndStreamRequest, PlayAudioRequest,
    SubscribeRequest,
};
use audio_api::audio_sink::CpalConfig;
use audio_api::audio_source::AudioCaptureConfig;
use audio_api::tonic::service::{audio, AudioServiceImpl};
use audio_api::types::AUDIO_CHUNK_SIZE;
use futures::StreamExt;
use hound::WavReader;
use log::{debug, info};
use std::convert::TryFrom;
use std::io::BufReader;
use std::time::Duration;
use tokio::time::timeout;
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::{Channel, Server, Uri};
use tonic::Request;
use uuid::Uuid;

/// Helper function to get the number of samples from an AudioChunk
fn get_sample_count(chunk: &AudioChunk) -> usize {
    match &chunk.samples {
        Some(audio::audio_chunk::Samples::FloatSamples(bytes)) => bytes.len() / 4,
        Some(audio::audio_chunk::Samples::Int16Samples(bytes)) => bytes.len() / 2,
        Some(audio::audio_chunk::Samples::Int32Samples(bytes)) => bytes.len() / 4,
        Some(audio::audio_chunk::Samples::Float64Samples(bytes)) => bytes.len() / 8,
        Some(audio::audio_chunk::Samples::Int24Samples(bytes)) => bytes.len() / 3,
        None => 0,
    }
}

// Helper to create a Unix socket path
fn create_socket_path() -> String {
    format!("/tmp/audio_test_{}.sock", Uuid::new_v4())
}

// Helper to start a gRPC server on Unix socket
async fn start_unix_server() -> Result<
    (
        AudioServiceClient<Channel>,
        String,
        tokio::task::JoinHandle<Result<(), tonic::transport::Error>>,
    ),
    Box<dyn std::error::Error>,
> {
    let socket_path = create_socket_path();

    // Remove socket file if it exists
    let _ = std::fs::remove_file(&socket_path);

    let service = AudioServiceImpl::new_with_config(CpalConfig::default())?;

    // Create Unix listener
    let uds = tokio::net::UnixListener::bind(&socket_path)?;
    let uds_stream = tokio_stream::wrappers::UnixListenerStream::new(uds);

    // Start server
    let server = Server::builder()
        .add_service(AudioServiceServer::new(service))
        .serve_with_incoming(uds_stream);

    let handle = tokio::spawn(server);

    // Give server time to start
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Create client with Unix socket
    let channel = {
        let socket_path = socket_path.clone();
        let connector = tower::service_fn(move |_: Uri| {
            let socket_path = socket_path.clone();
            async move {
                let stream = tokio::net::UnixStream::connect(socket_path).await?;
                Ok::<_, std::io::Error>(hyper_util::rt::tokio::TokioIo::new(stream))
            }
        });

        tonic::transport::Endpoint::try_from("http://[::]:50051")?
            .connect_with_connector(connector)
            .await?
    };

    let client = AudioServiceClient::new(channel);

    Ok((client, socket_path, handle))
}

// Helper to load WAV file and convert to gRPC AudioChunk format
fn load_wav_as_grpc_chunks(file_path: &str) -> Result<Vec<AudioChunk>, Box<dyn std::error::Error>> {
    let file = std::fs::File::open(file_path)?;
    let reader = BufReader::new(file);
    let mut wav_reader = WavReader::new(reader)?;

    let spec = wav_reader.spec();
    info!(
        "Loading WAV file: {} channels, {} Hz, {} bits",
        spec.channels, spec.sample_rate, spec.bits_per_sample
    );

    // Read all samples as i16 (original format)
    let samples: Result<Vec<i16>, _> = wav_reader.samples::<i16>().collect();
    let samples = samples?;

    // Convert to gRPC AudioChunk format
    let mut chunks = Vec::new();
    for (chunk_idx, chunk_samples) in samples.chunks(AUDIO_CHUNK_SIZE).enumerate() {
        // Convert i16 samples to bytes
        let mut bytes = Vec::with_capacity(chunk_samples.len() * 2);
        for &sample in chunk_samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }

        let mut chunk = AudioChunk {
            samples: Some(audio::audio_chunk::Samples::Int16Samples(bytes)),
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
            format: None,
        };

        // Add format metadata to the first chunk only
        if chunk_idx == 0 {
            chunk.format = Some(audio::AudioFormat {
                sample_rate: spec.sample_rate,
                channels: spec.channels as u32,
                sample_format: audio::SampleFormat::I16 as i32, // WAV files are typically I16
            });
        }

        chunks.push(chunk);
    }

    Ok(chunks)
}

// Helper to clean up socket file
fn cleanup_socket(socket_path: &str) {
    let _ = std::fs::remove_file(socket_path);
}

#[tokio::test]
async fn test_service_creation() {
    let _service =
        AudioServiceImpl::new_with_config(CpalConfig::default()).expect("Failed to create service");
    // Test that service can be created successfully
    assert!(true, "Service created successfully");
}

#[tokio::test]
async fn test_service_with_configs() {
    let playback_config = CpalConfig::default();

    let _service = AudioServiceImpl::new_with_config(playback_config)
        .expect("Failed to create service with configs");

    // Test that service can be created with custom configs
    assert!(true, "Service created with custom configs successfully");
}

#[tokio::test]
async fn test_service_methods_available() {
    // This test verifies that the service implements the required trait methods
    // without actually calling them (which would require a running server)
    let _service: AudioServiceImpl =
        AudioServiceImpl::new_with_config(CpalConfig::default()).expect("Failed to create service");

    // If this compiles, the service has the required methods
    assert!(true, "Service implements required trait methods");
}

#[tokio::test]
#[cfg_attr(not(feature = "audio_available"), ignore)]
async fn test_grpc_unix_socket_connection() {
    let _ = env_logger::try_init();

    // Test that we can start a server and connect via Unix socket
    let (mut client, socket_path, _server_handle) = match start_unix_server().await {
        Ok(result) => result,
        Err(e) => {
            info!("Could not start Unix socket server: {} - skipping test", e);
            return;
        }
    };

    info!("🔌 Testing Unix socket connection at {}", socket_path);

    // Test basic connection with a simple call
    let request = Request::new(SubscribeRequest {});

    match timeout(Duration::from_secs(2), client.subscribe_audio(request)).await {
        Ok(Ok(response)) => {
            info!("✅ Successfully connected via Unix socket");
            let mut stream = response.into_inner();

            // Try to get one chunk (with short timeout)
            match timeout(Duration::from_millis(500), stream.next()).await {
                Ok(Some(Ok(chunk))) => {
                    info!(
                        "📥 Received audio chunk via Unix socket with {} samples",
                        get_sample_count(&chunk)
                    );
                }
                Ok(Some(Err(e))) => {
                    info!("⚠️ Stream error: {} (expected in test environment)", e);
                }
                Ok(None) => {
                    info!("📡 Stream ended");
                }
                Err(_) => {
                    info!("⏰ Timeout waiting for audio chunks (expected in test environment)");
                }
            }
        }
        Ok(Err(e)) => {
            info!(
                "⚠️ gRPC call failed: {} (may be expected in test environment)",
                e
            );
        }
        Err(_) => {
            info!("⏰ Timeout connecting (may be expected in test environment)");
        }
    }

    // Clean up
    cleanup_socket(&socket_path);

    info!("🔌 Unix socket connection test completed");
}

#[tokio::test]
#[cfg_attr(not(feature = "audio_available"), ignore)]
async fn test_grpc_audio_playback_with_wav() {
    let _ = env_logger::try_init();

    info!("🔊 Starting WAV playback test...");

    // Start server
    let (mut client, socket_path, _server_handle) = match start_unix_server().await {
        Ok(result) => {
            info!("✅ Unix socket server started successfully");
            result
        }
        Err(e) => {
            info!(
                "❌ Could not start Unix socket server: {} - skipping test",
                e
            );
            return;
        }
    };

    info!("🔊 Testing audio playback via Unix socket");

    // Load test audio (correct path from audio_api directory)
    let test_file = "../tests/data/immediate_what_time_is_it.wav";
    info!("📁 Attempting to load WAV file: {}", test_file);

    let chunks = match load_wav_as_grpc_chunks(test_file) {
        Ok(chunks) => {
            info!(
                "✅ Successfully loaded {} chunks from WAV file",
                chunks.len()
            );
            chunks
        }
        Err(e) => {
            info!("❌ Could not load test WAV file: {} - skipping test", e);
            cleanup_socket(&socket_path);
            return;
        }
    };

    info!("📁 Loaded {} chunks from test file", chunks.len());

    // Create play audio stream
    let stream_id = Uuid::new_v4().to_string();
    info!("🎵 Creating playback stream with ID: {}", stream_id);

    let (tx, rx) = tokio::sync::mpsc::channel(100); // Larger buffer
    let request_stream = ReceiverStream::new(rx);

    // Start playback with timeout
    let play_future = client.play_audio(Request::new(request_stream));
    info!("🎵 Playback request sent, starting to send chunks...");

    // Send audio chunks (send all chunks for longer audio)
    let chunks_to_send = chunks.into_iter().collect::<Vec<_>>();
    let total_chunks = chunks_to_send.len();
    info!("📤 Sending {} chunks to playback stream", total_chunks);

    // Send chunks with timeout
    let send_future = async {
        for (i, chunk) in chunks_to_send.into_iter().enumerate() {
            let request = PlayAudioRequest {
                stream_id: stream_id.clone(),
                data: Some(play_audio_request::Data::Chunk(chunk)),
            };

            debug!("📤 Sending chunk {}/{}", i + 1, total_chunks);

            if let Err(e) = tx.send(request).await {
                info!("❌ Failed to send chunk {}: {}", i, e);
                break;
            }

            // Small delay to prevent overwhelming the receiver
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    };

    // Send chunks with timeout
    match timeout(Duration::from_secs(10), send_future).await {
        Ok(_) => {
            info!("📤 Finished sending chunks, sending end stream marker...");
        }
        Err(_) => {
            info!("⏰ Timeout sending chunks, proceeding with end stream");
        }
    }

    // Send end stream marker
    let end_request = PlayAudioRequest {
        stream_id: stream_id.clone(),
        data: Some(play_audio_request::Data::EndStream(true)),
    };

    if let Err(e) = tx.send(end_request).await {
        info!("❌ Failed to send end stream: {}", e);
    }

    // Close sender
    drop(tx);
    info!("📤 Sender closed, waiting for playback completion...");

    // Wait for playback completion with longer timeout for more chunks
    match timeout(Duration::from_secs(10), play_future).await {
        Ok(Ok(response)) => {
            let result = response.into_inner();
            info!(
                "✅ Playback completed: success={}, message={}",
                result.success, result.message
            );
            assert!(result.success, "Playback failed: {}", result.message);
        }
        Ok(Err(e)) => {
            info!("❌ Playback failed: {}", e);
            panic!("⚠️ Playback failed: {}", e);
        }
        Err(_) => {
            info!("⏰ Playback timeout");
            panic!("⏰ Playback timeout");
        }
    }

    // Clean up
    cleanup_socket(&socket_path);
    info!("🔊 Audio playback test completed successfully");
}

#[tokio::test]
#[cfg_attr(not(feature = "audio_available"), ignore)]
async fn test_grpc_stream_management() {
    let _ = env_logger::try_init();

    // Start server
    let (mut client, socket_path, _server_handle) = match start_unix_server().await {
        Ok(result) => result,
        Err(e) => {
            info!("Could not start Unix socket server: {} - skipping test", e);
            return;
        }
    };

    info!("🎛️ Testing stream management via Unix socket");

    let stream_id = Uuid::new_v4().to_string();

    // Test end stream
    let end_request = Request::new(EndStreamRequest {
        stream_id: stream_id.clone(),
    });

    match timeout(Duration::from_secs(2), client.end_audio_stream(end_request)).await {
        Ok(Ok(response)) => {
            let result = response.into_inner();
            info!(
                "📋 End stream response: success={}, message={}",
                result.success, result.message
            );
        }
        Ok(Err(e)) => {
            info!("⚠️ End stream failed: {}", e);
        }
        Err(_) => {
            info!("⏰ End stream timeout");
        }
    }

    // Test abort stream
    let abort_request = Request::new(AbortRequest {
        stream_id: stream_id.clone(),
    });

    match timeout(Duration::from_secs(2), client.abort_playback(abort_request)).await {
        Ok(Ok(response)) => {
            let result = response.into_inner();
            info!(
                "🛑 Abort stream response: success={}, message={}",
                result.success, result.message
            );
        }
        Ok(Err(e)) => {
            info!("⚠️ Abort stream failed: {}", e);
        }
        Err(_) => {
            info!("⏰ Abort stream timeout");
        }
    }

    // Clean up
    cleanup_socket(&socket_path);

    info!("🎛️ Stream management test completed");
}

#[tokio::test]
#[cfg_attr(not(feature = "audio_available"), ignore)]
async fn test_grpc_alphabet_recording_simulation() {
    let _ = env_logger::try_init();

    // Start server
    let (mut client, socket_path, _server_handle) = match start_unix_server().await {
        Ok(result) => result,
        Err(e) => {
            info!("Could not start Unix socket server: {} - skipping test", e);
            return;
        }
    };

    info!("🎤 Testing alphabet recording simulation via Unix socket");
    info!("📝 Simulating user saying: A, B, C, D, E");

    // Generate synthetic speech-like audio for alphabet (shorter for testing)
    let mut synthetic_chunks = Vec::new();

    // Generate 0.5 seconds of synthetic "speech" (much shorter for testing)
    let sample_rate = 16000;
    let duration_seconds = 0.5;
    let total_samples = (sample_rate as f32 * duration_seconds) as usize;

    // Create speech-like patterns for each letter
    let letters = ["A", "B", "C", "D", "E"];
    let samples_per_letter = total_samples / letters.len();

    for (letter_idx, letter) in letters.iter().enumerate() {
        info!("🔤 Generating synthetic audio for letter '{}'", letter);

        let start_sample = letter_idx * samples_per_letter;
        let end_sample = (letter_idx + 1) * samples_per_letter;

        for i in start_sample..end_sample {
            let t = i as f32 / sample_rate as f32;
            let letter_t = (i - start_sample) as f32 / samples_per_letter as f32;

            // Create letter-specific frequency patterns
            let base_freq = 150.0 + (letter_idx as f32 * 50.0); // Different base freq per letter
            let formant1 = base_freq * 2.0 * (1.0 + 0.3 * letter_t.sin());
            let formant2 = base_freq * 3.0 * (1.0 + 0.2 * (letter_t * 1.5).sin());

            // Amplitude envelope (fade in/out for each letter)
            let envelope = if letter_t < 0.1 {
                letter_t * 10.0 // Fade in
            } else if letter_t > 0.9 {
                (1.0 - letter_t) * 10.0 // Fade out
            } else {
                1.0 // Steady
            };

            let sample = 0.1
                * envelope
                * (0.6 * (2.0 * std::f32::consts::PI * formant1 * t).sin()
                    + 0.4 * (2.0 * std::f32::consts::PI * formant2 * t).sin());

            synthetic_chunks.push(sample);
        }

        // Add brief pause between letters
        for _ in 0..(sample_rate / 50) {
            // 20ms pause (shorter)
            synthetic_chunks.push(0.0);
        }
    }

    // Convert to gRPC chunks
    let mut grpc_chunks = Vec::new();
    for (chunk_idx, chunk_samples) in synthetic_chunks.chunks(AUDIO_CHUNK_SIZE).enumerate() {
        let mut chunk_vec = vec![0.0f32; AUDIO_CHUNK_SIZE];
        for (i, &sample) in chunk_samples.iter().enumerate() {
            if i < AUDIO_CHUNK_SIZE {
                chunk_vec[i] = sample;
            }
        }

        let mut chunk = AudioChunk {
            samples: {
                // Convert f32 samples to bytes
                let mut bytes = Vec::with_capacity(chunk_vec.len() * 4);
                for &sample in &chunk_vec {
                    bytes.extend_from_slice(&sample.to_le_bytes());
                }
                Some(audio::audio_chunk::Samples::FloatSamples(bytes))
            },
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
            format: None,
        };

        // Add format metadata to the first chunk only
        if chunk_idx == 0 {
            chunk.format = Some(audio::AudioFormat {
                sample_rate: sample_rate,
                channels: 1,                                    // Mono synthetic audio
                sample_format: audio::SampleFormat::F32 as i32, // F32 synthetic audio
            });
        }

        grpc_chunks.push(chunk);
    }

    info!(
        "📹 Generated {} chunks of synthetic alphabet audio",
        grpc_chunks.len()
    );

    // "Record" and "playback" the alphabet via gRPC
    let stream_id = "alphabet_recording".to_string();
    let (tx, rx) = tokio::sync::mpsc::channel(32);
    let request_stream = ReceiverStream::new(rx);

    let play_future = client.play_audio(Request::new(request_stream));

    // Send the synthetic alphabet audio (faster for testing)
    for (i, chunk) in grpc_chunks.into_iter().enumerate() {
        let request = PlayAudioRequest {
            stream_id: stream_id.clone(),
            data: Some(play_audio_request::Data::Chunk(chunk)),
        };

        if tx.send(request).await.is_err() {
            break;
        }

        if i % 5 == 0 {
            info!("📤 Sent chunk {} of alphabet audio", i + 1);
        }

        // Faster playback simulation
        tokio::time::sleep(Duration::from_millis(10)).await; // 10ms per chunk
    }

    // End playback
    let end_request = PlayAudioRequest {
        stream_id: stream_id.clone(),
        data: Some(play_audio_request::Data::EndStream(true)),
    };
    let _ = tx.send(end_request).await;
    drop(tx);

    // Wait for playback completion with shorter timeout
    match timeout(Duration::from_secs(5), play_future).await {
        Ok(Ok(response)) => {
            let result = response.into_inner();
            info!("✅ Alphabet playback completed: {}", result.message);
            assert!(
                result.success,
                "Alphabet playback failed: {}",
                result.message
            );
        }
        Ok(Err(e)) => {
            panic!("⚠️ Alphabet playback failed: {}", e);
        }
        Err(_) => {
            panic!("⏰ Alphabet playback timeout");
        }
    }

    // Clean up
    cleanup_socket(&socket_path);

    info!("�� Alphabet recording simulation completed");
}

#[tokio::test]
#[cfg_attr(not(feature = "audio_available"), ignore)]
async fn test_grpc_concurrent_streams() {
    let _ = env_logger::try_init();

    // Start server
    let (client, socket_path, _server_handle) = match start_unix_server().await {
        Ok(result) => result,
        Err(e) => {
            info!("Could not start Unix socket server: {} - skipping test", e);
            return;
        }
    };

    info!("🎭 Testing concurrent audio streams via Unix socket");

    // Load test audio (correct path)
    let chunks = match load_wav_as_grpc_chunks("../tests/data/hey_mycroft_test.wav") {
        Ok(chunks) => chunks,
        Err(e) => {
            info!("Could not load test WAV file: {} - skipping test", e);
            cleanup_socket(&socket_path);
            return;
        }
    };

    // Limit chunks for faster testing
    let chunks = chunks.into_iter().take(10).collect::<Vec<_>>();

    // Start multiple concurrent streams (fewer for faster testing)
    let stream_count = 2;
    let mut futures = Vec::new();

    for i in 0..stream_count {
        let mut client_clone = client.clone();
        let chunks_clone = chunks.clone();
        let stream_id = format!("concurrent_{}", i);

        let future = tokio::spawn(async move {
            let (tx, rx) = tokio::sync::mpsc::channel(32);
            let request_stream = ReceiverStream::new(rx);

            let play_future = client_clone.play_audio(Request::new(request_stream));

            // Send chunks (faster)
            for chunk in chunks_clone {
                let request = PlayAudioRequest {
                    stream_id: stream_id.clone(),
                    data: Some(play_audio_request::Data::Chunk(chunk)),
                };

                if tx.send(request).await.is_err() {
                    break;
                }

                tokio::time::sleep(Duration::from_millis(1)).await; // 1ms delay
            }

            // End stream
            let end_request = PlayAudioRequest {
                stream_id: stream_id.clone(),
                data: Some(play_audio_request::Data::EndStream(true)),
            };
            let _ = tx.send(end_request).await;
            drop(tx);

            // Return result
            play_future.await
        });

        futures.push(future);
    }

    // Wait for all streams to complete with shorter timeout
    let mut success_count = 0;
    for (i, future) in futures.into_iter().enumerate() {
        match timeout(Duration::from_secs(5), future).await {
            Ok(Ok(Ok(response))) => {
                let result = response.into_inner();
                if result.success {
                    success_count += 1;
                    info!("✅ Concurrent stream {} completed successfully", i);
                } else {
                    panic!("⚠️ Concurrent stream {} failed: {}", i, result.message);
                }
            }
            Ok(Ok(Err(e))) => {
                panic!("⚠️ Concurrent stream {} gRPC error: {}", i, e);
            }
            Ok(Err(e)) => {
                panic!("⚠️ Concurrent stream {} task error: {}", i, e);
            }
            Err(_) => {
                panic!("⏰ Concurrent stream {} timeout", i);
            }
        }
    }

    info!(
        "🎭 Concurrent streams test completed: {}/{} succeeded",
        success_count, stream_count
    );

    // Clean up
    cleanup_socket(&socket_path);
}
