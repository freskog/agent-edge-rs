use crate::protocol::{Connection, Message, ProtocolError};
use log::{debug, error, info, trace, warn};
use std::collections::VecDeque;
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// High-level TCP client for the audio API
pub struct AudioClient {
    connection: Connection,
    server_address: String,
}

impl AudioClient {
    /// Connect to the audio server
    pub fn connect(address: &str) -> Result<Self, ProtocolError> {
        info!("📡 Connecting to audio server at {}", address);

        let stream = TcpStream::connect(address)?;
        stream.set_read_timeout(Some(Duration::from_secs(30)))?;
        stream.set_write_timeout(Some(Duration::from_secs(10)))?;

        let connection = Connection::new(stream)?;

        info!("✅ Connected to audio server");

        Ok(AudioClient {
            connection,
            server_address: address.to_string(),
        })
    }

    /// Subscribe to audio capture stream
    pub fn subscribe_audio(&mut self) -> Result<(), ProtocolError> {
        debug!("📤 Sending SubscribeAudio message");

        let message = Message::SubscribeAudio;
        self.connection.write_message(&message)?;

        info!("🎤 Subscribed to audio capture");
        Ok(())
    }

    /// Unsubscribe from audio capture
    pub fn unsubscribe_audio(&mut self) -> Result<UnsubscribeResult, ProtocolError> {
        debug!("📤 Sending UnsubscribeAudio message");

        let message = Message::UnsubscribeAudio;
        self.connection.write_message(&message)?;

        let response = self.connection.read_message()?;

        match response {
            Message::UnsubscribeResponse { success, message } => {
                if success {
                    info!("✅ Unsubscribed from audio: {}", message);
                } else {
                    error!("❌ Unsubscribe failed: {}", message);
                }

                Ok(UnsubscribeResult { success, message })
            }
            Message::ErrorResponse { message } => {
                error!("❌ Server error: {}", message);
                Ok(UnsubscribeResult {
                    success: false,
                    message,
                })
            }
            other => {
                error!("❌ Unexpected message type: {:?}", other.message_type());
                Err(ProtocolError::InvalidMessageType(other.message_type() as u8))
            }
        }
    }

    /// Read a single audio chunk (blocking, may return None if no data available)
    pub fn read_audio_chunk(&mut self) -> Result<Option<AudioChunk>, ProtocolError> {
        let message = self.connection.read_message()?;

        match message {
            Message::AudioChunk {
                audio_data,
                timestamp_ms,
            } => {
                trace!(
                    "📥 Received audio chunk: {} bytes (timestamp: {})",
                    audio_data.len(),
                    timestamp_ms
                );

                Ok(Some(AudioChunk {
                    data: audio_data,
                    timestamp_ms,
                }))
            }
            Message::ErrorResponse { message } => {
                error!("❌ Server error: {}", message);
                Ok(None)
            }
            other => {
                error!("❌ Unexpected message type: {:?}", other.message_type());
                Err(ProtocolError::InvalidMessageType(other.message_type() as u8))
            }
        }
    }

    /// Play audio chunk and wait for response
    pub fn play_audio_chunk(
        &mut self,
        stream_id: &str,
        audio_data: Vec<u8>,
    ) -> Result<PlayResult, ProtocolError> {
        debug!(
            "📤 Sending audio chunk: {} bytes to stream '{}'",
            audio_data.len(),
            stream_id
        );

        let message = Message::PlayAudio {
            stream_id: stream_id.to_string(),
            audio_data,
        };

        self.connection.write_message(&message)?;

        // Wait for response, but handle any unexpected delayed messages
        loop {
            let response = self.connection.read_message()?;

            match response {
                Message::PlayResponse { success, message } => {
                    if success {
                        debug!("✅ Play response: {}", message);
                    } else {
                        error!("❌ Play failed: {}", message);
                    }

                    return Ok(PlayResult { success, message });
                }
                Message::ErrorResponse { message } => {
                    error!("❌ Server error: {}", message);
                    return Ok(PlayResult {
                        success: false,
                        message,
                    });
                }
                Message::UnsubscribeResponse { .. } => {
                    // Drain delayed UnsubscribeResponse messages that arrived after we started playback
                    debug!("🔄 Draining delayed UnsubscribeResponse during playback");
                    continue;
                }
                other => {
                    error!(
                        "❌ Unexpected response type during playback: {:?}",
                        other.message_type()
                    );
                    return Err(ProtocolError::InvalidMessageType(other.message_type() as u8));
                }
            }
        }
    }

    /// End audio stream and wait for completion
    pub fn end_stream(&mut self, stream_id: &str) -> Result<EndStreamResult, ProtocolError> {
        debug!("📤 Ending stream '{}'", stream_id);

        let message = Message::EndStream {
            stream_id: stream_id.to_string(),
        };

        self.connection.write_message(&message)?;

        let response = self.connection.read_message()?;

        match response {
            Message::EndStreamResponse {
                success,
                message,
                chunks_played,
            } => {
                if success {
                    info!(
                        "✅ Stream ended: {} (played {} chunks)",
                        message, chunks_played
                    );
                } else {
                    error!("❌ End stream failed: {}", message);
                }

                Ok(EndStreamResult {
                    success,
                    message,
                    chunks_played,
                })
            }
            Message::ErrorResponse { message } => {
                error!("❌ Server error: {}", message);
                Ok(EndStreamResult {
                    success: false,
                    message,
                    chunks_played: 0,
                })
            }
            other => {
                error!("❌ Unexpected message type: {:?}", other.message_type());
                Err(ProtocolError::InvalidMessageType(other.message_type() as u8))
            }
        }
    }

    /// Abort audio stream playback and wait for response
    pub fn abort_stream(&mut self, stream_id: &str) -> Result<AbortResult, ProtocolError> {
        debug!("📤 Aborting stream '{}'", stream_id);

        let message = Message::AbortPlayback {
            stream_id: stream_id.to_string(),
        };

        self.connection.write_message(&message)?;

        // Wait for response, but handle any unexpected delayed messages
        loop {
            let response = self.connection.read_message()?;

            match response {
                Message::AbortResponse { success, message } => {
                    if success {
                        info!("✅ Playback aborted: {}", message);
                    } else {
                        error!("❌ Abort failed: {}", message);
                    }

                    return Ok(AbortResult { success, message });
                }
                Message::ErrorResponse { message } => {
                    error!("❌ Server error: {}", message);
                    return Ok(AbortResult {
                        success: false,
                        message,
                    });
                }
                Message::PlayResponse { .. } => {
                    // Drain delayed PlayResponse messages
                    debug!("🔄 Draining delayed PlayResponse during abort");
                    continue;
                }
                other => {
                    error!(
                        "❌ Unexpected response type during abort: {:?}",
                        other.message_type()
                    );
                    return Err(ProtocolError::InvalidMessageType(other.message_type() as u8));
                }
            }
        }
    }

    /// Get the server address
    pub fn server_address(&self) -> &str {
        &self.server_address
    }

    /// Set the underlying connection to non-blocking mode
    pub fn set_nonblocking(&mut self, nonblocking: bool) -> Result<(), ProtocolError> {
        self.connection.set_nonblocking(nonblocking)
    }

    /// Read an audio chunk with a timeout
    /// Returns Ok(Some(chunk)) if chunk received, Ok(None) if timeout, Err for real errors
    pub fn read_audio_chunk_timeout(
        &mut self,
        timeout: std::time::Duration,
    ) -> Result<Option<AudioChunk>, ProtocolError> {
        use std::io::ErrorKind;
        use std::time::Instant;

        // Set to non-blocking mode
        self.set_nonblocking(true)?;

        let start = Instant::now();

        loop {
            match self.read_audio_chunk() {
                Ok(Some(chunk)) => {
                    // Success - restore blocking mode and return chunk
                    self.set_nonblocking(false)?;
                    return Ok(Some(chunk));
                }
                Ok(None) => {
                    // No chunk available, continue trying
                    if start.elapsed() >= timeout {
                        self.set_nonblocking(false)?;
                        return Ok(None);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                    continue;
                }
                Err(ProtocolError::Io(e)) if e.kind() == ErrorKind::WouldBlock => {
                    // No data available yet
                    if start.elapsed() >= timeout {
                        // Timeout - restore blocking mode and return None
                        self.set_nonblocking(false)?;
                        return Ok(None);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                    continue;
                }
                Err(e) => {
                    // Real error - restore blocking mode and return error
                    self.set_nonblocking(false)?;
                    return Err(e);
                }
            }
        }
    }
}

/// Buffered audio client that provides continuous streaming with internal buffering
///
/// This client maintains a rolling buffer of audio chunks and provides a simple
/// streaming interface that hides buffering complexity from consumers.
pub struct BufferedAudioClient {
    server_address: String,
    buffer: Arc<Mutex<AudioBuffer>>,
    _background_thread: thread::JoinHandle<()>,
}

#[derive(Debug)]
struct AudioBuffer {
    chunks: VecDeque<AudioChunk>,
    max_buffer_duration_ms: u64,
    total_chunks_received: u64,
    total_chunks_dropped: u64,
    last_chunk_time: Option<Instant>,
}

impl AudioBuffer {
    fn new(max_buffer_duration_ms: u64) -> Self {
        Self {
            chunks: VecDeque::new(),
            max_buffer_duration_ms,
            total_chunks_received: 0,
            total_chunks_dropped: 0,
            last_chunk_time: None,
        }
    }

    fn add_chunk(&mut self, chunk: AudioChunk) {
        self.total_chunks_received += 1;
        self.last_chunk_time = Some(Instant::now());
        self.chunks.push_back(chunk);

        // Remove old chunks to maintain buffer size
        self.trim_buffer();
    }

    fn trim_buffer(&mut self) {
        if self.chunks.is_empty() {
            return;
        }

        let newest_timestamp = self.chunks.back().unwrap().timestamp_ms;
        let cutoff_timestamp = newest_timestamp.saturating_sub(self.max_buffer_duration_ms);

        let initial_len = self.chunks.len();
        self.chunks
            .retain(|chunk| chunk.timestamp_ms >= cutoff_timestamp);
        let dropped = initial_len - self.chunks.len();

        if dropped > 0 {
            self.total_chunks_dropped += dropped as u64;
            debug!("🗑️ Dropped {} old audio chunks from buffer", dropped);
        }
    }

    fn get_next_chunk(&mut self) -> Option<AudioChunk> {
        self.chunks.pop_front()
    }

    fn get_stats(&self) -> BufferStats {
        BufferStats {
            current_chunks: self.chunks.len(),
            total_received: self.total_chunks_received,
            total_dropped: self.total_chunks_dropped,
            buffer_duration_ms: if let (Some(front), Some(back)) =
                (self.chunks.front(), self.chunks.back())
            {
                back.timestamp_ms.saturating_sub(front.timestamp_ms)
            } else {
                0
            },
            last_chunk_age_ms: self
                .last_chunk_time
                .map(|t| t.elapsed().as_millis() as u64)
                .unwrap_or(u64::MAX),
        }
    }
}

impl BufferedAudioClient {
    /// Create a new buffered audio client with specified buffer duration
    pub fn connect(address: &str, buffer_duration_ms: u64) -> Result<Self, ProtocolError> {
        info!(
            "🎧 Connecting buffered audio client to {} (buffer: {}ms)",
            address, buffer_duration_ms
        );

        let buffer = Arc::new(Mutex::new(AudioBuffer::new(buffer_duration_ms)));
        let buffer_clone = Arc::clone(&buffer);
        let address_clone = address.to_string();

        // Start background thread for continuous audio reading
        let background_thread = thread::spawn(move || {
            if let Err(e) = Self::background_audio_loop(address_clone, buffer_clone) {
                error!("❌ Background audio thread failed: {}", e);
            }
        });

        // Give the background thread a moment to establish connection
        thread::sleep(Duration::from_millis(100));

        Ok(Self {
            server_address: address.to_string(),
            buffer,
            _background_thread: background_thread,
        })
    }

    /// Create with default 2-second buffer (optimal for wakeword context)
    pub fn connect_default(address: &str) -> Result<Self, ProtocolError> {
        Self::connect(address, 2000) // 2 seconds
    }

    fn background_audio_loop(
        address: String,
        buffer: Arc<Mutex<AudioBuffer>>,
    ) -> Result<(), ProtocolError> {
        info!("🎤 Starting background audio capture thread");

        let mut client = AudioClient::connect(&address)?;
        client.subscribe_audio()?;

        loop {
            match client.read_audio_chunk() {
                Ok(Some(chunk)) => {
                    if let Ok(mut buffer_guard) = buffer.lock() {
                        buffer_guard.add_chunk(chunk);
                    } else {
                        warn!("⚠️ Failed to acquire buffer lock, dropping chunk");
                    }
                }
                Ok(None) => {
                    debug!("📭 No audio chunk available");
                    thread::sleep(Duration::from_millis(10));
                }
                Err(e) => {
                    error!("❌ Failed to read audio chunk: {}", e);
                    thread::sleep(Duration::from_millis(100));
                    // Could implement reconnection logic here
                }
            }
        }
    }

    /// Get the next available audio chunk (non-blocking)
    /// Returns None if no chunks are currently available
    pub fn read_chunk(&self) -> Option<AudioChunk> {
        self.buffer
            .lock()
            .ok()
            .and_then(|mut buffer| buffer.get_next_chunk())
    }

    /// Get the next available audio chunk (blocking with timeout)
    /// Waits up to the specified duration for a chunk to become available
    pub fn read_chunk_timeout(&self, timeout: Duration) -> Option<AudioChunk> {
        let start = Instant::now();

        while start.elapsed() < timeout {
            if let Some(chunk) = self.read_chunk() {
                return Some(chunk);
            }
            thread::sleep(Duration::from_millis(1));
        }

        None
    }

    /// Get current buffer statistics
    pub fn get_stats(&self) -> Option<BufferStats> {
        self.buffer.lock().ok().map(|buffer| buffer.get_stats())
    }

    /// Get server address
    pub fn server_address(&self) -> &str {
        &self.server_address
    }
}

/// Statistics about the buffered audio client
#[derive(Debug, Clone)]
pub struct BufferStats {
    pub current_chunks: usize,
    pub total_received: u64,
    pub total_dropped: u64,
    pub buffer_duration_ms: u64,
    pub last_chunk_age_ms: u64,
}

impl BufferStats {
    pub fn is_healthy(&self) -> bool {
        // Buffer is healthy if:
        // 1. We have recent data (< 100ms old)
        // 2. Drop rate is reasonable (< 5%)
        self.last_chunk_age_ms < 100
            && (self.total_received == 0 || (self.total_dropped * 100 / self.total_received) < 5)
    }

    pub fn log_status(&self) {
        let drop_rate = if self.total_received > 0 {
            (self.total_dropped * 100 / self.total_received) as f32
        } else {
            0.0
        };

        info!(
            "📊 Audio Buffer: {} chunks ({:.1}s), {}/{} received/dropped ({:.1}%), last chunk {}ms ago",
            self.current_chunks,
            self.buffer_duration_ms as f32 / 1000.0,
            self.total_received,
            self.total_dropped,
            drop_rate,
            self.last_chunk_age_ms
        );
    }
}

/// Audio chunk received from the server
#[derive(Debug, Clone)]
pub struct AudioChunk {
    pub data: Vec<u8>,
    pub timestamp_ms: u64,
}

impl AudioChunk {
    /// Get the size of the audio data in bytes
    pub fn size_bytes(&self) -> usize {
        self.data.len()
    }

    /// Get the number of samples (assuming s16le format)
    pub fn sample_count(&self) -> usize {
        self.data.len() / 2
    }

    /// Get the duration of this chunk in milliseconds (assuming 16kHz s16le)
    pub fn duration_ms(&self) -> f32 {
        self.sample_count() as f32 / 16.0 // 16kHz sample rate
    }
}

/// Result of a play audio operation
#[derive(Debug, Clone)]
pub struct PlayResult {
    pub success: bool,
    pub message: String,
}

/// Result of an end stream operation
#[derive(Debug, Clone)]
pub struct EndStreamResult {
    pub success: bool,
    pub message: String,
    pub chunks_played: u32,
}

/// Result of an unsubscribe operation
#[derive(Debug, Clone)]
pub struct UnsubscribeResult {
    pub success: bool,
    pub message: String,
}

/// Result of an abort stream operation
#[derive(Debug, Clone)]
pub struct AbortResult {
    pub success: bool,
    pub message: String,
}
