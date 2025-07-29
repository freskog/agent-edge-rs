use audio_protocol::client::AudioClient;
use audio_protocol::AudioChunk;
use crossbeam_channel::{bounded, Receiver, Sender};
use std::collections::VecDeque;
use std::error::Error;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Request types for the SharedAudioClient
#[derive(Debug)]
pub enum AudioRequest {
    /// Get the most recent N seconds of buffered audio
    GetRecentAudio {
        seconds: u32,
        response: Sender<Vec<AudioChunk>>,
    },
    /// Start streaming live audio to the provided sender
    /// Will send chunks for the specified duration, then send EndMarker
    StartLiveStream {
        sender: Sender<AudioMessage>,
        duration: Duration,
    },
    /// Get current buffer statistics
    GetStats { response: Sender<AudioStats> },
    /// Shutdown the audio client
    Shutdown,
}

/// Messages sent over live audio streams
#[derive(Debug, Clone)]
pub enum AudioMessage {
    /// Regular audio chunk
    Chunk(AudioChunk),
    /// Signals end of the audio stream
    EndMarker,
}

/// Statistics about the audio buffer
#[derive(Debug, Clone)]
pub struct AudioStats {
    pub buffer_chunks: usize,
    pub buffer_duration_ms: u64,
    pub total_chunks_received: u64,
    pub total_chunks_dropped: u64,
    pub last_chunk_age_ms: u64,
    pub is_connected: bool,
}

impl AudioStats {
    pub fn is_healthy(&self) -> bool {
        self.is_connected && self.last_chunk_age_ms < 200 // Less than 200ms old
    }
}

/// Shared audio client that manages a background audio connection
/// and provides a message-passing API for audio access
pub struct SharedAudioClient {
    request_sender: Sender<AudioRequest>,
    background_handle: Option<JoinHandle<()>>,
}

impl SharedAudioClient {
    /// Create a new SharedAudioClient that connects to the audio server
    pub fn new(audio_address: String) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let (request_sender, request_receiver) = bounded(32);

        // Test connection first before spawning background thread
        {
            let mut test_client = AudioClient::connect(&audio_address)?;
            test_client.subscribe_audio()?;
            // Connection successful, drop the test client
        }

        // Now spawn background thread with a working connection
        let audio_address_clone = audio_address.clone();
        let background_handle = thread::spawn(move || {
            if let Err(e) = Self::run_background_loop(audio_address_clone, request_receiver) {
                log::error!("❌ SharedAudioClient background thread failed: {}", e);
            }
        });

        Ok(Self {
            request_sender,
            background_handle: Some(background_handle),
        })
    }

    /// Get recent audio from the buffer (blocking call)
    pub fn get_recent_audio(
        &self,
        seconds: u32,
    ) -> Result<Vec<AudioChunk>, Box<dyn std::error::Error + Send + Sync>> {
        let (response_sender, response_receiver) = bounded(1);

        self.request_sender.send(AudioRequest::GetRecentAudio {
            seconds,
            response: response_sender,
        })?;

        Ok(response_receiver.recv_timeout(Duration::from_secs(5))?)
    }

    /// Start a live audio stream for the specified duration
    pub fn start_live_stream(
        &self,
        duration: Duration,
    ) -> Result<Receiver<AudioMessage>, Box<dyn std::error::Error + Send + Sync>> {
        let (stream_sender, stream_receiver) = bounded(256);

        self.request_sender.send(AudioRequest::StartLiveStream {
            sender: stream_sender,
            duration,
        })?;

        Ok(stream_receiver)
    }

    /// Get current buffer statistics
    pub fn get_stats(&self) -> Result<AudioStats, Box<dyn std::error::Error + Send + Sync>> {
        let (response_sender, response_receiver) = bounded(1);

        self.request_sender.send(AudioRequest::GetStats {
            response: response_sender,
        })?;

        Ok(response_receiver.recv_timeout(Duration::from_secs(1))?)
    }

    /// Background loop that manages the audio connection and buffer
    fn run_background_loop(
        audio_address: String,
        request_receiver: Receiver<AudioRequest>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        log::info!("🎤 Starting SharedAudioClient background loop");

        // Connect to audio server
        let mut audio_client = AudioClient::connect(&audio_address)?;
        audio_client.subscribe_audio()?;

        // Set connection to non-blocking mode so we can check for shutdown signals
        audio_client.set_nonblocking(true)?;

        log::info!("✅ SharedAudioClient connected to {}", audio_address);

        // Audio buffer (ring buffer for last N seconds)
        let mut audio_buffer = VecDeque::new();
        let max_buffer_duration = Duration::from_secs(5); // Keep 5 seconds of audio
        let mut stats = AudioStats {
            buffer_chunks: 0,
            buffer_duration_ms: 0,
            total_chunks_received: 0,
            total_chunks_dropped: 0,
            last_chunk_age_ms: 0,
            is_connected: true,
        };

        // Active live streams
        let mut live_streams: Vec<(Sender<AudioMessage>, Instant, Duration)> = Vec::new();

        loop {
            // Check for new requests (non-blocking)
            if let Ok(request) = request_receiver.try_recv() {
                match request {
                    AudioRequest::GetRecentAudio { seconds, response } => {
                        let chunks = Self::get_recent_chunks(&audio_buffer, seconds);
                        let _ = response.send(chunks);
                    }
                    AudioRequest::StartLiveStream { sender, duration } => {
                        live_streams.push((sender, Instant::now(), duration));
                        log::debug!("🎵 Started live stream for {:?}", duration);
                    }
                    AudioRequest::GetStats { response } => {
                        let _ = response.send(stats.clone());
                    }
                    AudioRequest::Shutdown => {
                        log::info!("🔚 SharedAudioClient shutting down");
                        break;
                    }
                }
            }

            // Read audio chunk (now truly non-blocking)
            match audio_client.read_audio_chunk() {
                Ok(Some(chunk)) => {
                    stats.total_chunks_received += 1;
                    stats.last_chunk_age_ms = 0;

                    // Add to buffer
                    audio_buffer.push_back((chunk.clone(), Instant::now()));

                    // Trim buffer to max duration
                    Self::trim_buffer(&mut audio_buffer, max_buffer_duration);
                    stats.buffer_chunks = audio_buffer.len();
                    stats.buffer_duration_ms = Self::calculate_buffer_duration(&audio_buffer);

                    // Send to active live streams
                    Self::send_to_live_streams(&mut live_streams, &chunk, &mut stats);
                }
                Ok(None) => {
                    // No audio available right now
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(e) => {
                    // Check if it's just a "would block" error (expected with non-blocking)
                    if let Some(io_err) =
                        e.source().and_then(|e| e.downcast_ref::<std::io::Error>())
                    {
                        if io_err.kind() == std::io::ErrorKind::WouldBlock {
                            // No data available, continue
                            std::thread::sleep(Duration::from_millis(1));
                            continue;
                        }
                    }

                    log::error!("❌ Failed to read audio chunk: {}", e);
                    stats.is_connected = false;
                    std::thread::sleep(Duration::from_millis(100));
                    // Could implement reconnection logic here
                }
            }

            // Update chunk ages
            if let Some((_, oldest_time)) = audio_buffer.front() {
                stats.last_chunk_age_ms = oldest_time.elapsed().as_millis() as u64;
            }

            // Clean up expired live streams
            Self::cleanup_expired_streams(&mut live_streams);
        }

        Ok(())
    }

    fn get_recent_chunks(
        buffer: &VecDeque<(AudioChunk, Instant)>,
        seconds: u32,
    ) -> Vec<AudioChunk> {
        let target_duration = Duration::from_secs(seconds as u64);
        let now = Instant::now();

        buffer
            .iter()
            .rev() // Start from most recent
            .take_while(|(_, timestamp)| now.duration_since(*timestamp) <= target_duration)
            .map(|(chunk, _)| chunk.clone())
            .collect::<Vec<_>>()
            .into_iter()
            .rev() // Restore chronological order
            .collect()
    }

    fn trim_buffer(buffer: &mut VecDeque<(AudioChunk, Instant)>, max_duration: Duration) {
        let now = Instant::now();
        while let Some((_, timestamp)) = buffer.front() {
            if now.duration_since(*timestamp) > max_duration {
                buffer.pop_front();
            } else {
                break;
            }
        }
    }

    fn calculate_buffer_duration(buffer: &VecDeque<(AudioChunk, Instant)>) -> u64 {
        buffer
            .iter()
            .map(|(chunk, _)| chunk.duration_ms() as u64)
            .sum()
    }

    fn send_to_live_streams(
        streams: &mut Vec<(Sender<AudioMessage>, Instant, Duration)>,
        chunk: &AudioChunk,
        stats: &mut AudioStats,
    ) {
        streams.retain(|(sender, start_time, duration)| {
            if start_time.elapsed() < *duration {
                // Stream is still active
                match sender.try_send(AudioMessage::Chunk(chunk.clone())) {
                    Ok(_) => true, // Keep stream
                    Err(_) => {
                        stats.total_chunks_dropped += 1;
                        false // Remove stream (receiver disconnected)
                    }
                }
            } else {
                // Stream duration expired - send end marker
                let _ = sender.try_send(AudioMessage::EndMarker);
                log::debug!("🔚 Live stream duration expired, sent EndMarker");
                false // Remove stream
            }
        });
    }

    fn cleanup_expired_streams(streams: &mut Vec<(Sender<AudioMessage>, Instant, Duration)>) {
        // This is handled in send_to_live_streams, but we could add additional cleanup here
    }
}

impl Drop for SharedAudioClient {
    fn drop(&mut self) {
        // Send shutdown signal
        let _ = self.request_sender.send(AudioRequest::Shutdown);

        // Wait for background thread to finish
        if let Some(handle) = self.background_handle.take() {
            let _ = handle.join();
        }
    }
}
