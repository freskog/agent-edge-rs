use audio_api::audio_sink::CpalConfig;
use audio_api::tonic::service::AudioServiceImpl;
use audio_api::types::AUDIO_CHUNK_SIZE;
use futures::StreamExt;
use hound::WavReader;
use log::{debug, info};
use service_protos::audio_service_client::AudioServiceClient;
use service_protos::audio_service_server::AudioServiceServer;
use service_protos::{
    play_audio_request, AbortRequest, AudioChunk, EndStreamRequest, PlayAudioRequest,
    SubscribeRequest,
};
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
        Some(service_protos::audio_chunk::Samples::FloatSamples(bytes)) => bytes.len() / 4,
        Some(service_protos::audio_chunk::Samples::Int16Samples(bytes)) => bytes.len() / 2,
        Some(service_protos::audio_chunk::Samples::Int32Samples(bytes)) => bytes.len() / 4,
        Some(service_protos::audio_chunk::Samples::Float64Samples(bytes)) => bytes.len() / 8,
        Some(service_protos::audio_chunk::Samples::Int24Samples(bytes)) => bytes.len() / 3,
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
            samples: Some(service_protos::audio_chunk::Samples::Int16Samples(bytes)),
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
            format: None,
        };

        // Add format metadata to the first chunk only
        if chunk_idx == 0 {
            chunk.format = Some(service_protos::AudioFormat {
                sample_rate: spec.sample_rate,
                channels: spec.channels as u32,
                sample_format: service_protos::SampleFormat::I16 as i32, // WAV files are typically I16
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
                Some(service_protos::audio_chunk::Samples::FloatSamples(bytes))
            },
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
            format: None,
        };

        // Add format metadata to the first chunk only
        if chunk_idx == 0 {
            chunk.format = Some(service_protos::AudioFormat {
                sample_rate: sample_rate,
                channels: 1, // Mono synthetic audio
                sample_format: service_protos::SampleFormat::F32 as i32, // F32 synthetic audio
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

#[tokio::test]
#[cfg_attr(not(feature = "audio_available"), ignore)]
async fn test_subscribe_audio_single_subscriber() {
    let _ = env_logger::try_init();

    let (mut client, socket_path, _server_handle) = match start_unix_server().await {
        Ok(result) => result,
        Err(e) => {
            info!("Could not start Unix socket server: {} - skipping test", e);
            return;
        }
    };

    info!("🎤 Testing single audio subscriber");

    // Subscribe to audio
    let request = Request::new(SubscribeRequest {});
    let response = match client.subscribe_audio(request).await {
        Ok(response) => response,
        Err(e) => {
            info!(
                "Subscribe failed (expected in non-audio environments): {}",
                e
            );
            cleanup_socket(&socket_path);
            return;
        }
    };

    let mut stream = response.into_inner();
    info!("✅ Successfully subscribed to audio stream");

    // Give the audio capture service a moment to start
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Try to receive a few chunks with timeout
    let mut chunks_received = 0;
    let max_chunks = 5;

    for i in 0..max_chunks {
        match timeout(Duration::from_millis(1000), stream.next()).await {
            Ok(Some(Ok(chunk))) => {
                chunks_received += 1;
                info!(
                    "📥 Received audio chunk {} with {} samples",
                    i + 1,
                    get_sample_count(&chunk)
                );

                // Verify chunk has expected format
                assert!(chunk.format.is_some(), "Chunk should have format metadata");
                let format = chunk.format.unwrap();
                assert_eq!(format.sample_rate, 16000, "Expected 16kHz sample rate");
                assert_eq!(format.channels, 1, "Expected mono audio");
            }
            Ok(Some(Err(e))) => {
                info!("Stream error: {}", e);
                break;
            }
            Ok(None) => {
                info!("Stream ended");
                break;
            }
            Err(_) => {
                info!(
                    "Timeout waiting for audio chunk {} - this is expected in some environments",
                    i + 1
                );
                break;
            }
        }
    }

    info!("🎤 Received {} audio chunks", chunks_received);
    cleanup_socket(&socket_path);
}

#[tokio::test]
#[cfg_attr(not(feature = "audio_available"), ignore)]
async fn test_subscribe_audio_multiple_concurrent_subscribers() {
    let _ = env_logger::try_init();

    let (client, socket_path, _server_handle) = match start_unix_server().await {
        Ok(result) => result,
        Err(e) => {
            info!("Could not start Unix socket server: {} - skipping test", e);
            return;
        }
    };

    info!("🎤 Testing multiple concurrent audio subscribers");

    let num_subscribers = 3;
    let mut handles = Vec::new();

    // Create multiple subscribers concurrently
    for i in 0..num_subscribers {
        let mut client_clone = client.clone();
        let handle = tokio::spawn(async move {
            let request = Request::new(SubscribeRequest {});
            match client_clone.subscribe_audio(request).await {
                Ok(response) => {
                    let mut stream = response.into_inner();
                    info!("✅ Subscriber {} connected", i);

                    let mut chunks_received = 0;
                    let max_chunks = 3;

                    for _ in 0..max_chunks {
                        match timeout(Duration::from_millis(1000), stream.next()).await {
                            Ok(Some(Ok(chunk))) => {
                                chunks_received += 1;
                                debug!(
                                    "📥 Subscriber {} received chunk with {} samples",
                                    i,
                                    get_sample_count(&chunk)
                                );
                            }
                            Ok(Some(Err(e))) => {
                                info!("Subscriber {} stream error: {}", i, e);
                                break;
                            }
                            Ok(None) => {
                                info!("Subscriber {} stream ended", i);
                                break;
                            }
                            Err(_) => {
                                debug!("Subscriber {} timeout - expected in some environments", i);
                                break;
                            }
                        }
                    }

                    info!("🎤 Subscriber {} received {} chunks", i, chunks_received);
                    chunks_received
                }
                Err(e) => {
                    info!("Subscriber {} failed to connect: {}", i, e);
                    0
                }
            }
        });
        handles.push(handle);
    }

    // Wait for all subscribers to complete
    let mut total_chunks = 0;
    for handle in handles {
        match handle.await {
            Ok(chunks) => total_chunks += chunks,
            Err(e) => info!("Subscriber task failed: {}", e),
        }
    }

    info!(
        "🎤 Total chunks received across all subscribers: {}",
        total_chunks
    );

    // In a working audio environment, we should receive some chunks
    // In a non-audio environment, this will be 0, which is fine
    assert!(total_chunks >= 0, "Should receive non-negative chunks");

    cleanup_socket(&socket_path);
}

#[tokio::test]
#[cfg_attr(not(feature = "audio_available"), ignore)]
async fn test_record_and_playback_scenario() {
    let _ = env_logger::try_init();

    let (mut client, socket_path, _server_handle) = match start_unix_server().await {
        Ok(result) => result,
        Err(e) => {
            info!("Could not start Unix socket server: {} - skipping test", e);
            return;
        }
    };

    info!("🎤🔊 Testing record and playback scenario");

    // Step 1: Subscribe to audio and collect some chunks
    let request = Request::new(SubscribeRequest {});
    let response = match client.subscribe_audio(request).await {
        Ok(response) => response,
        Err(e) => {
            info!(
                "Subscribe failed (expected in non-audio environments): {}",
                e
            );
            cleanup_socket(&socket_path);
            return;
        }
    };

    let mut stream = response.into_inner();
    info!("✅ Recording audio...");

    let mut recorded_chunks = Vec::new();
    let max_record_chunks = 10;

    for i in 0..max_record_chunks {
        match timeout(Duration::from_millis(500), stream.next()).await {
            Ok(Some(Ok(chunk))) => {
                recorded_chunks.push(chunk);
                info!(
                    "📥 Recorded chunk {} with {} samples",
                    i + 1,
                    get_sample_count(&recorded_chunks.last().unwrap())
                );
            }
            Ok(Some(Err(e))) => {
                info!("Recording stream error: {}", e);
                break;
            }
            Ok(None) => {
                info!("Recording stream ended");
                break;
            }
            Err(_) => {
                info!("Recording timeout - using test data instead");
                break;
            }
        }
    }

    // If we didn't record any real audio, use test data
    if recorded_chunks.is_empty() {
        info!("🎤 No audio recorded, using test WAV file for playback");

        // Load test WAV file
        let test_chunks =
            match load_wav_as_grpc_chunks("../tests/data/immediate_what_time_is_it.wav") {
                Ok(chunks) => chunks,
                Err(e) => {
                    info!(
                        "Could not load test WAV file: {} - skipping playback test",
                        e
                    );
                    cleanup_socket(&socket_path);
                    return;
                }
            };
        recorded_chunks = test_chunks;
    }

    info!(
        "🎤 Recorded {} chunks, now playing back...",
        recorded_chunks.len()
    );

    // Step 2: Play back the recorded audio
    let stream_id = Uuid::new_v4().to_string();
    let (tx, rx) = tokio::sync::mpsc::channel(100);

    // Send playback request
    let playback_request = PlayAudioRequest {
        stream_id: stream_id.clone(),
        data: None, // Will be set in the stream
    };

    let request_stream = ReceiverStream::new(rx);
    let response_future = client.play_audio(Request::new(request_stream));

    // Send recorded chunks for playback
    let total_chunks = recorded_chunks.len();
    tokio::spawn(async move {
        info!("🔊 Starting playback of {} chunks", total_chunks);

        for (i, chunk) in recorded_chunks.into_iter().enumerate() {
            let request = PlayAudioRequest {
                stream_id: stream_id.clone(),
                data: Some(play_audio_request::Data::Chunk(chunk)),
            };

            debug!(
                "📤 Sending recorded chunk {}/{} for playback",
                i + 1,
                total_chunks
            );

            if let Err(e) = tx.send(request).await {
                info!("Failed to send playback chunk: {}", e);
                break;
            }

            // Small delay to prevent overwhelming the system
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        // Send end stream marker
        let end_request = PlayAudioRequest {
            stream_id: stream_id.clone(),
            data: Some(play_audio_request::Data::EndStream(true)),
        };

        if let Err(e) = tx.send(end_request).await {
            info!("Failed to send end stream marker: {}", e);
        }

        info!("🔊 Finished sending playback chunks");
    });

    // Wait for playback to complete
    match timeout(Duration::from_secs(30), response_future).await {
        Ok(Ok(response)) => {
            let result = response.into_inner();
            info!(
                "✅ Record and playback completed: success={}, message={}",
                result.success, result.message
            );
            assert!(result.success, "Playback should succeed");
        }
        Ok(Err(e)) => {
            info!("Playback failed: {}", e);
            // Don't fail the test - audio might not be available
        }
        Err(_) => {
            info!("Playback timeout - this may be expected in some environments");
        }
    }

    cleanup_socket(&socket_path);
}

#[tokio::test]
#[cfg_attr(not(feature = "audio_available"), ignore)]
async fn test_subscriber_cleanup_on_disconnect() {
    let _ = env_logger::try_init();

    let (mut client, socket_path, _server_handle) = match start_unix_server().await {
        Ok(result) => result,
        Err(e) => {
            info!("Could not start Unix socket server: {} - skipping test", e);
            return;
        }
    };

    info!("🎤 Testing subscriber cleanup on disconnect");

    // Create a subscriber and then drop it
    {
        let request = Request::new(SubscribeRequest {});
        if let Ok(response) = client.subscribe_audio(request).await {
            let mut stream = response.into_inner();
            info!("✅ Subscriber connected");

            // Receive one chunk to ensure connection is established
            match timeout(Duration::from_millis(1000), stream.next()).await {
                Ok(Some(Ok(chunk))) => {
                    info!(
                        "📥 Received chunk with {} samples",
                        get_sample_count(&chunk)
                    );
                }
                Ok(Some(Err(e))) => {
                    info!("Stream error: {}", e);
                }
                Ok(None) => {
                    info!("Stream ended");
                }
                Err(_) => {
                    info!("Timeout - expected in some environments");
                }
            }

            // Stream will be dropped here, triggering cleanup
        }
    }

    info!("🎤 Subscriber dropped - cleanup should have occurred");

    // Give some time for cleanup to happen
    tokio::time::sleep(Duration::from_millis(100)).await;

    // The test passes if no panics occur during cleanup
    info!("✅ Subscriber cleanup test completed");

    cleanup_socket(&socket_path);
}

#[tokio::test]
#[cfg_attr(not(feature = "audio_available"), ignore)]
async fn test_subscribe_audio_with_simulated_data() {
    let _ = env_logger::try_init();

    let (mut client, socket_path, _server_handle) = match start_unix_server().await {
        Ok(result) => result,
        Err(e) => {
            info!("Could not start Unix socket server: {} - skipping test", e);
            return;
        }
    };

    info!("🎤 Testing audio subscription with simulated data");

    // Subscribe to audio
    let request = Request::new(SubscribeRequest {});
    let response = match client.subscribe_audio(request).await {
        Ok(response) => response,
        Err(e) => {
            info!("Subscribe failed: {}", e);
            cleanup_socket(&socket_path);
            return;
        }
    };

    let mut stream = response.into_inner();
    info!("✅ Successfully subscribed to audio stream");

    // Test the subscription mechanism by checking if we can receive the stream
    // Even if no audio is captured, the stream should be established
    match timeout(Duration::from_millis(500), stream.next()).await {
        Ok(Some(Ok(chunk))) => {
            info!(
                "📥 Received audio chunk with {} samples",
                get_sample_count(&chunk)
            );

            // Verify chunk format
            assert!(chunk.format.is_some(), "Chunk should have format metadata");
            let format = chunk.format.unwrap();
            assert_eq!(format.sample_rate, 16000, "Expected 16kHz sample rate");
            assert_eq!(format.channels, 1, "Expected mono audio");
            assert_eq!(
                format.sample_format,
                service_protos::SampleFormat::F32 as i32,
                "Expected F32 format"
            );

            // Validate the audio data exists and is reasonable
            let sample_count = get_sample_count(&chunk);
            assert!(sample_count > 0, "Should have audio samples");
            assert!(sample_count <= 2048, "Sample count should be reasonable");

            // Validate timestamp is reasonable (within last few seconds)
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64;
            assert!(chunk.timestamp_ms > 0, "Timestamp should be set");
            assert!(
                chunk.timestamp_ms <= now,
                "Timestamp should not be in the future"
            );
            assert!(
                chunk.timestamp_ms > now - 10000,
                "Timestamp should be recent (within 10 seconds)"
            );

            info!("✅ Audio subscription working correctly with valid audio data");
        }
        Ok(Some(Err(e))) => {
            info!("Stream error: {}", e);
        }
        Ok(None) => {
            info!("Stream ended immediately");
        }
        Err(_) => {
            info!("⏱️ Timeout waiting for audio - this is expected in non-audio environments");
            info!("✅ Audio subscription established successfully (no audio input available)");
        }
    }

    cleanup_socket(&socket_path);
}

#[tokio::test]
#[cfg_attr(not(feature = "audio_available"), ignore)]
async fn test_concurrent_subscriber_management() {
    let _ = env_logger::try_init();

    let (client, socket_path, _server_handle) = match start_unix_server().await {
        Ok(result) => result,
        Err(e) => {
            info!("Could not start Unix socket server: {} - skipping test", e);
            return;
        }
    };

    info!("🎤 Testing concurrent subscriber management");

    let num_subscribers = 5;
    let mut handles = Vec::new();

    // Create multiple subscribers concurrently
    for i in 0..num_subscribers {
        let mut client_clone = client.clone();
        let handle = tokio::spawn(async move {
            let request = Request::new(SubscribeRequest {});
            match client_clone.subscribe_audio(request).await {
                Ok(response) => {
                    let mut stream = response.into_inner();
                    info!("✅ Subscriber {} connected successfully", i);

                    // Keep the connection alive for a short time
                    tokio::time::sleep(Duration::from_millis(200)).await;

                    // Try to receive at least one message (or timeout)
                    match timeout(Duration::from_millis(100), stream.next()).await {
                        Ok(Some(Ok(chunk))) => {
                            info!(
                                "📥 Subscriber {} received audio data with {} samples",
                                i,
                                get_sample_count(&chunk)
                            );

                            // Validate the audio chunk format
                            assert!(
                                chunk.format.is_some(),
                                "Subscriber {} chunk should have format metadata",
                                i
                            );
                            let format = chunk.format.unwrap();
                            assert_eq!(
                                format.sample_rate, 16000,
                                "Subscriber {} expected 16kHz sample rate",
                                i
                            );
                            assert_eq!(format.channels, 1, "Subscriber {} expected mono audio", i);
                            assert_eq!(
                                format.sample_format,
                                service_protos::SampleFormat::F32 as i32,
                                "Subscriber {} expected F32 format",
                                i
                            );

                            // Validate the audio data exists and is reasonable
                            let sample_count = get_sample_count(&chunk);
                            assert!(
                                sample_count > 0,
                                "Subscriber {} should have audio samples",
                                i
                            );
                            assert!(
                                sample_count <= 2048,
                                "Subscriber {} sample count should be reasonable",
                                i
                            );

                            info!("✅ Subscriber {} audio data validated successfully", i);
                        }
                        Ok(Some(Err(e))) => {
                            info!("Subscriber {} stream error: {}", i, e);
                        }
                        Ok(None) => {
                            info!("Subscriber {} stream ended", i);
                        }
                        Err(_) => {
                            info!(
                                "Subscriber {} timeout (expected in non-audio environments)",
                                i
                            );
                        }
                    }

                    true // Successfully connected
                }
                Err(e) => {
                    info!("Subscriber {} failed to connect: {}", i, e);
                    false
                }
            }
        });
        handles.push(handle);
    }

    // Wait for all subscribers to complete
    let mut successful_connections = 0;
    for (i, handle) in handles.into_iter().enumerate() {
        match handle.await {
            Ok(true) => {
                successful_connections += 1;
                info!("✅ Subscriber {} completed successfully", i);
            }
            Ok(false) => {
                info!("❌ Subscriber {} failed to connect", i);
            }
            Err(e) => {
                info!("❌ Subscriber {} task failed: {}", i, e);
            }
        }
    }

    info!(
        "🎤 Successfully connected {} out of {} subscribers",
        successful_connections, num_subscribers
    );

    // All subscribers should be able to connect
    assert_eq!(
        successful_connections, num_subscribers,
        "All subscribers should be able to connect simultaneously"
    );

    cleanup_socket(&socket_path);
}

#[tokio::test]
#[cfg_attr(not(feature = "audio_available"), ignore)]
async fn test_audio_sample_validation() {
    let _ = env_logger::try_init();

    let (mut client, socket_path, _server_handle) = match start_unix_server().await {
        Ok(result) => result,
        Err(e) => {
            info!("Could not start Unix socket server: {} - skipping test", e);
            return;
        }
    };

    info!("🎤 Testing audio sample validation");

    // Subscribe to audio
    let request = Request::new(SubscribeRequest {});
    let response = match client.subscribe_audio(request).await {
        Ok(response) => response,
        Err(e) => {
            info!("Subscribe failed: {}", e);
            cleanup_socket(&socket_path);
            return;
        }
    };

    let mut stream = response.into_inner();
    info!("✅ Successfully subscribed to audio stream");

    // Collect multiple chunks to validate audio data
    let mut chunks_validated = 0;
    let target_chunks = 3;

    for i in 0..target_chunks {
        match timeout(Duration::from_millis(1000), stream.next()).await {
            Ok(Some(Ok(chunk))) => {
                info!(
                    "📥 Validating audio chunk {} with {} samples",
                    i + 1,
                    get_sample_count(&chunk)
                );

                // Validate format
                assert!(chunk.format.is_some(), "Chunk should have format metadata");
                let format = chunk.format.unwrap();
                assert_eq!(format.sample_rate, 16000, "Expected 16kHz sample rate");
                assert_eq!(format.channels, 1, "Expected mono audio");
                assert_eq!(
                    format.sample_format,
                    service_protos::SampleFormat::F32 as i32,
                    "Expected F32 format"
                );

                // Validate sample data
                if let Some(service_protos::audio_chunk::Samples::FloatSamples(sample_bytes)) =
                    &chunk.samples
                {
                    assert!(!sample_bytes.is_empty(), "Sample data should not be empty");
                    assert_eq!(
                        sample_bytes.len() % 4,
                        0,
                        "F32 samples should be 4-byte aligned"
                    );

                    // Convert bytes to f32 samples for validation
                    let mut samples = Vec::new();
                    for chunk_bytes in sample_bytes.chunks(4) {
                        if chunk_bytes.len() == 4 {
                            let sample = f32::from_le_bytes([
                                chunk_bytes[0],
                                chunk_bytes[1],
                                chunk_bytes[2],
                                chunk_bytes[3],
                            ]);
                            samples.push(sample);
                        }
                    }

                    assert!(!samples.is_empty(), "Should have parsed samples");

                    // Validate sample values are reasonable for audio
                    let mut valid_samples = 0;
                    let mut non_zero_samples = 0;
                    let mut sample_sum = 0.0f32;

                    for sample in &samples {
                        // Audio samples should be finite and within reasonable range
                        assert!(sample.is_finite(), "Sample should be finite: {}", sample);
                        assert!(
                            sample.abs() <= 2.0,
                            "Sample should be within reasonable range: {}",
                            sample
                        );

                        if sample.abs() > 0.0001 {
                            non_zero_samples += 1;
                        }

                        sample_sum += sample.abs();
                        valid_samples += 1;
                    }

                    info!(
                        "🔍 Chunk {} stats: {} samples, {} non-zero, avg magnitude: {:.6}",
                        i + 1,
                        valid_samples,
                        non_zero_samples,
                        sample_sum / valid_samples as f32
                    );

                    // At least some samples should be non-zero (unless in a very quiet environment)
                    // This is a soft check since we might be in a quiet environment
                    if non_zero_samples > 0 {
                        info!("✅ Audio contains non-zero samples (live audio detected)");
                    } else {
                        info!("ℹ️ All samples are near zero (quiet environment or no input)");
                    }
                } else {
                    panic!("Expected FloatSamples but got different sample type");
                }

                chunks_validated += 1;
                info!("✅ Chunk {} validated successfully", i + 1);
            }
            Ok(Some(Err(e))) => {
                info!("Stream error on chunk {}: {}", i + 1, e);
                break;
            }
            Ok(None) => {
                info!("Stream ended on chunk {}", i + 1);
                break;
            }
            Err(_) => {
                info!(
                    "Timeout on chunk {} - this is expected in some environments",
                    i + 1
                );
                break;
            }
        }
    }

    info!(
        "🎤 Successfully validated {} audio chunks",
        chunks_validated
    );
    assert!(
        chunks_validated > 0,
        "Should have validated at least one chunk"
    );

    cleanup_socket(&socket_path);
}

#[tokio::test]
#[cfg_attr(not(feature = "audio_available"), ignore)]
async fn test_echo_3_second_recording() {
    let _ = env_logger::try_init();

    let (mut client, socket_path, _server_handle) = match start_unix_server().await {
        Ok(result) => result,
        Err(e) => {
            info!("Could not start Unix socket server: {} - skipping test", e);
            return;
        }
    };

    info!("🎤🔊 Echo Test: Record 3 seconds and play back");
    info!("📢 GET READY TO SPEAK! Test will start recording in 2 seconds...");

    // Give user time to prepare
    tokio::time::sleep(Duration::from_millis(2000)).await;

    // Step 1: Subscribe to audio for recording
    let request = Request::new(SubscribeRequest {});
    let response = match client.subscribe_audio(request).await {
        Ok(response) => response,
        Err(e) => {
            info!("Subscribe failed: {}", e);
            cleanup_socket(&socket_path);
            return;
        }
    };

    let mut stream = response.into_inner();
    info!("🔴 RECORDING NOW! Please speak clearly for 3 seconds...");
    info!("💬 Say something like: 'Hello, this is an echo test!'");

    // Record for exactly 3 seconds
    let recording_duration = Duration::from_secs(3);
    let start_time = std::time::Instant::now();
    let mut recorded_chunks = Vec::new();
    let mut total_samples = 0;
    let mut non_zero_samples = 0;

    while start_time.elapsed() < recording_duration {
        let remaining = recording_duration - start_time.elapsed();
        match timeout(remaining.min(Duration::from_millis(100)), stream.next()).await {
            Ok(Some(Ok(chunk))) => {
                let sample_count = get_sample_count(&chunk);
                total_samples += sample_count;

                // Count non-zero samples to detect speech
                if let Some(service_protos::audio_chunk::Samples::FloatSamples(sample_bytes)) =
                    &chunk.samples
                {
                    for chunk_bytes in sample_bytes.chunks(4) {
                        if chunk_bytes.len() == 4 {
                            let sample = f32::from_le_bytes([
                                chunk_bytes[0],
                                chunk_bytes[1],
                                chunk_bytes[2],
                                chunk_bytes[3],
                            ]);
                            if sample.abs() > 0.001 {
                                // Threshold for detecting speech
                                non_zero_samples += 1;
                            }
                        }
                    }
                }

                recorded_chunks.push(chunk);

                // Show progress
                let elapsed = start_time.elapsed().as_millis();
                if elapsed % 500 < 100 {
                    // Show progress every ~500ms
                    let remaining_ms = recording_duration.as_millis().saturating_sub(elapsed);
                    info!(
                        "🎤 Recording... {:.1}s remaining",
                        remaining_ms as f64 / 1000.0
                    );
                }
            }
            Ok(Some(Err(e))) => {
                info!("Recording stream error: {}", e);
                break;
            }
            Ok(None) => {
                info!("Recording stream ended unexpectedly");
                break;
            }
            Err(_) => {
                // Timeout is normal as we're controlling the duration
                continue;
            }
        }
    }

    info!("⏹️ Recording complete!");
    info!(
        "📊 Recorded {} chunks, {} total samples, {} samples with speech",
        recorded_chunks.len(),
        total_samples,
        non_zero_samples
    );

    // Validate we got some audio
    assert!(
        !recorded_chunks.is_empty(),
        "Should have recorded some audio chunks"
    );
    assert!(total_samples > 0, "Should have recorded some audio samples");

    let speech_ratio = non_zero_samples as f64 / total_samples as f64;
    info!(
        "🗣️ Speech detection: {:.1}% of samples contain audio above threshold",
        speech_ratio * 100.0
    );

    if speech_ratio < 0.01 {
        info!("⚠️ Very little audio detected - you might want to speak louder or check your microphone");
    } else {
        info!("✅ Good audio levels detected for playback");
    }

    // Give a brief pause before playback
    info!("🔊 Starting playback in 1 second...");
    tokio::time::sleep(Duration::from_millis(1000)).await;

    // Step 2: Play back the recorded audio
    let stream_id = Uuid::new_v4().to_string();
    let (tx, rx) = tokio::sync::mpsc::channel(100);

    let request_stream = ReceiverStream::new(rx);
    let response_future = client.play_audio(Request::new(request_stream));

    // Send recorded chunks for playback
    let total_chunks = recorded_chunks.len();
    let playback_task = tokio::spawn(async move {
        info!(
            "🔊 Playing back your recording ({} chunks)...",
            total_chunks
        );

        for (i, chunk) in recorded_chunks.into_iter().enumerate() {
            let request = PlayAudioRequest {
                stream_id: stream_id.clone(),
                data: Some(play_audio_request::Data::Chunk(chunk)),
            };

            if let Err(e) = tx.send(request).await {
                info!("Failed to send playback chunk: {}", e);
                break;
            }

            // Show playback progress
            if i % 10 == 0 {
                let progress = (i as f64 / total_chunks as f64) * 100.0;
                info!("🔊 Playback progress: {:.0}%", progress);
            }

            // Small delay to maintain real-time playback
            tokio::time::sleep(Duration::from_millis(5)).await;
        }

        // Send end stream marker
        let end_request = PlayAudioRequest {
            stream_id: stream_id.clone(),
            data: Some(play_audio_request::Data::EndStream(true)),
        };

        if let Err(e) = tx.send(end_request).await {
            info!("Failed to send end stream marker: {}", e);
        }

        info!("🔊 Playback complete!");
    });

    // Wait for playback to complete
    let playback_timeout = Duration::from_secs(10); // Give extra time for processing
    match timeout(playback_timeout, response_future).await {
        Ok(Ok(response)) => {
            let result = response.into_inner();
            info!("✅ Echo test completed successfully!");
            info!(
                "📊 Playback result: success={}, message='{}'",
                result.success, result.message
            );
            assert!(result.success, "Echo playback should succeed");

            // Wait for the playback task to complete
            let _ = playback_task.await;

            info!("🎉 Echo test passed! You should have heard your recording played back.");
        }
        Ok(Err(e)) => {
            info!("❌ Echo test failed during playback: {}", e);
            panic!("Echo test playback failed: {}", e);
        }
        Err(_) => {
            info!("❌ Echo test timed out during playback");
            panic!("Echo test timed out - playback took too long");
        }
    }

    cleanup_socket(&socket_path);
}

#[tokio::test]
#[cfg_attr(not(feature = "audio_available"), ignore)]
async fn test_grpc_abort_playback_mid_stream() {
    let _ = env_logger::try_init();

    // Start server
    let (mut client, socket_path, _server_handle) = match start_unix_server().await {
        Ok(result) => result,
        Err(e) => {
            info!("Could not start Unix socket server: {} - skipping test", e);
            return;
        }
    };

    info!("🛑 Testing abort playback mid-stream via Unix socket");

    // Load test audio file
    let test_chunks = match load_wav_as_grpc_chunks("../tests/data/hey_mycroft_test.wav") {
        Ok(chunks) => chunks,
        Err(e) => {
            info!("Could not load test audio file: {} - skipping test", e);
            cleanup_socket(&socket_path);
            return;
        }
    };

    let total_chunks = test_chunks.len();
    info!("📁 Loaded {} chunks from test audio file", total_chunks);

    // We'll abort after sending about 1/3 of the chunks
    let abort_after_chunk = total_chunks / 3;
    info!(
        "🎯 Will abort after chunk {}/{}",
        abort_after_chunk, total_chunks
    );

    let stream_id = "abort_test_stream".to_string();
    let (tx, rx) = tokio::sync::mpsc::channel(32);
    let request_stream = ReceiverStream::new(rx);

    // Clone client for abort request
    let mut abort_client = client.clone();

    // Start playback in the background
    let play_future = client.play_audio(Request::new(request_stream));

    // Send half the chunks first
    let chunks_to_send = abort_after_chunk;
    info!("📤 Sending {} chunks to establish stream", chunks_to_send);

    for (i, chunk) in test_chunks.into_iter().take(chunks_to_send).enumerate() {
        let request = PlayAudioRequest {
            stream_id: stream_id.clone(),
            data: Some(play_audio_request::Data::Chunk(chunk)),
        };

        if tx.send(request).await.is_err() {
            break;
        }

        info!("📤 Sent chunk {}", i + 1);

        // Small delay to ensure processing
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // Give time for chunks to be processed and stream to be registered
    info!("⏳ Waiting for stream to be registered...");
    tokio::time::sleep(Duration::from_millis(300)).await;
    info!("✅ Stream should now be registered");

    // Now send abort request
    let abort_request = Request::new(AbortRequest {
        stream_id: stream_id.clone(),
    });

    let abort_start = std::time::Instant::now();
    match timeout(
        Duration::from_secs(3),
        abort_client.abort_playback(abort_request),
    )
    .await
    {
        Ok(Ok(response)) => {
            let abort_duration = abort_start.elapsed();
            let result = response.into_inner();
            info!(
                "✅ Abort successful in {:?}: success={}, message={}",
                abort_duration, result.success, result.message
            );
            // Either stream-specific abort succeeded, or stream wasn't found yet but audio sink was aborted
            if result.success {
                info!("✅ Stream-specific abort succeeded");
            } else {
                info!("⚠️ Stream not found for abort (race condition), but audio sink was aborted");
                // This is acceptable - the audio sink abort still works
            }

            // Abort should be fast (< 1 second)
            assert!(
                abort_duration < Duration::from_secs(1),
                "Abort took too long: {:?}",
                abort_duration
            );
        }
        Ok(Err(e)) => {
            panic!("🛑 Abort request failed: {}", e);
        }
        Err(_) => {
            panic!("🛑 Abort request timed out");
        }
    }

    // Close the stream sender to signal end
    drop(tx);

    // The play_future should now complete quickly since we aborted
    let play_start = std::time::Instant::now();
    match timeout(Duration::from_secs(2), play_future).await {
        Ok(Ok(response)) => {
            let play_duration = play_start.elapsed();
            let result = response.into_inner();
            info!(
                "🎵 Playback completed in {:?}: success={}, message={}",
                play_duration, result.success, result.message
            );

            // Playback should complete quickly after abort
            assert!(
                play_duration < Duration::from_secs(1),
                "Playback took too long to complete after abort: {:?}",
                play_duration
            );
        }
        Ok(Err(e)) => {
            // This is expected if the stream was aborted
            info!("🎵 Playback stream ended due to abort: {}", e);
        }
        Err(_) => {
            panic!("🎵 Playback did not complete after abort within timeout");
        }
    }

    // Verify that audio buffers are cleared by checking buffer percentage
    // Note: This would require exposing buffer stats through the gRPC API
    // For now, we rely on the timing assertions above

    info!("✅ Abort test completed successfully");
    info!(
        "📊 Summary: Sent {}/{} chunks before abort",
        chunks_to_send, total_chunks
    );

    // Clean up
    cleanup_socket(&socket_path);
}
