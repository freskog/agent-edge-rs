use hound::{WavSpec, WavWriter};
use log::{debug, error, warn};
use rodio::{OutputStream, Sink, Source};
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::sync::Mutex;

#[derive(Error, Debug, Clone)]
pub enum AudioError {
    #[error("Failed to write audio data: {0}")]
    WriteError(String),

    #[error("Failed to stop audio playback: {0}")]
    StopError(String),

    #[error("Buffer full")]
    BufferFull,

    #[error("Failed to create WAV file: {0}")]
    WavCreationError(String),

    #[error("MP3 decoding not implemented")]
    Mp3DecodingNotImplemented,

    #[error("Base64 decode error: {0}")]
    Base64DecodeError(String),

    #[error("Invalid JSON: {0}")]
    InvalidJson(String),

    #[error("Missing field: {0}")]
    MissingField(String),

    #[error("Failed to save audio: {0}")]
    FailedToSaveAudio(String),
}

/// Core trait for audio output handling
#[async_trait::async_trait]
pub trait AudioSink: Send + Sync {
    /// Write audio data to the sink. The data is expected to be
    /// 16-bit PCM at 16kHz mono.
    async fn write(&self, audio_data: &[u8]) -> Result<(), AudioError>;

    /// Stop audio playback and clear any buffered data
    async fn stop(&self) -> Result<(), AudioError>;
}

/// A test implementation of AudioSink that stores all written data
/// and provides methods to inspect its state
#[derive(Clone)]
pub struct TestSink {
    chunks: Arc<Mutex<Vec<Vec<u8>>>>,
    is_stopped: Arc<AtomicBool>,
    total_bytes: Arc<AtomicUsize>,
    write_count: Arc<AtomicUsize>,
}

impl TestSink {
    pub fn new() -> Self {
        Self {
            chunks: Arc::new(Mutex::new(Vec::new())),
            is_stopped: Arc::new(AtomicBool::new(false)),
            total_bytes: Arc::new(AtomicUsize::new(0)),
            write_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Get all audio chunks that have been written
    pub async fn get_chunks(&self) -> Vec<Vec<u8>> {
        self.chunks.lock().await.clone()
    }

    /// Get the total number of bytes written
    pub fn total_bytes(&self) -> usize {
        self.total_bytes.load(Ordering::Acquire)
    }

    /// Get the number of write operations performed
    pub fn write_count(&self) -> usize {
        self.write_count.load(Ordering::Acquire)
    }

    /// Check if the sink has been stopped
    pub fn is_stopped(&self) -> bool {
        self.is_stopped.load(Ordering::Acquire)
    }

    /// Clear all stored data and reset counters
    pub async fn reset(&self) {
        self.chunks.lock().await.clear();
        self.is_stopped.store(false, Ordering::Release);
        self.total_bytes.store(0, Ordering::Release);
        self.write_count.store(0, Ordering::Release);
    }
}

#[async_trait::async_trait]
impl AudioSink for TestSink {
    async fn write(&self, audio_data: &[u8]) -> Result<(), AudioError> {
        if self.is_stopped() {
            return Err(AudioError::WriteError("Sink is stopped".to_string()));
        }

        let mut chunks = self.chunks.lock().await;
        chunks.push(audio_data.to_vec());

        self.total_bytes
            .fetch_add(audio_data.len(), Ordering::Release);
        self.write_count.fetch_add(1, Ordering::Release);

        Ok(())
    }

    async fn stop(&self) -> Result<(), AudioError> {
        self.is_stopped.store(true, Ordering::Release);
        Ok(())
    }
}

impl Default for TestSink {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_sink_write_and_inspect() {
        let sink = TestSink::new();

        // Write some test data
        let chunk1 = vec![1, 2, 3, 4];
        let chunk2 = vec![5, 6, 7, 8];

        sink.write(&chunk1).await.unwrap();
        sink.write(&chunk2).await.unwrap();

        // Verify state
        assert_eq!(sink.write_count(), 2);
        assert_eq!(sink.total_bytes(), 8);

        let chunks = sink.get_chunks().await;
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0], chunk1);
        assert_eq!(chunks[1], chunk2);
    }

    #[tokio::test]
    async fn test_sink_stop() {
        let sink = TestSink::new();

        // Write before stopping
        sink.write(&[1, 2, 3, 4]).await.unwrap();

        // Stop the sink
        sink.stop().await.unwrap();
        assert!(sink.is_stopped());

        // Verify write fails after stop
        let result = sink.write(&[5, 6, 7, 8]).await;
        assert!(result.is_err());
        if let Err(AudioError::WriteError(msg)) = result {
            assert_eq!(msg, "Sink is stopped");
        } else {
            panic!("Expected WriteError");
        }
    }

    #[tokio::test]
    async fn test_sink_reset() {
        let sink = TestSink::new();

        // Write some data and stop
        sink.write(&[1, 2, 3, 4]).await.unwrap();
        sink.stop().await.unwrap();

        // Reset the sink
        sink.reset().await;

        // Verify state is cleared
        assert_eq!(sink.write_count(), 0);
        assert_eq!(sink.total_bytes(), 0);
        assert!(!sink.is_stopped());
        assert!(sink.get_chunks().await.is_empty());

        // Verify can write after reset
        assert!(sink.write(&[5, 6, 7, 8]).await.is_ok());
    }
}

/// A sink that writes audio to a WAV file
pub struct WavFileSink {
    writer: Arc<Mutex<WavWriter<BufWriter<File>>>>,
    is_stopped: Arc<AtomicBool>,
}

impl WavFileSink {
    /// Create a new WAV file sink
    /// The file will be configured for 16kHz 16-bit mono PCM
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self, AudioError> {
        let spec = WavSpec {
            channels: 1,
            sample_rate: 16000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };

        let writer = WavWriter::create(path, spec).map_err(|e| {
            AudioError::WavCreationError(format!("Failed to create WAV file: {}", e))
        })?;

        Ok(Self {
            writer: Arc::new(Mutex::new(writer)),
            is_stopped: Arc::new(AtomicBool::new(false)),
        })
    }
}

#[async_trait::async_trait]
impl AudioSink for WavFileSink {
    async fn write(&self, audio_data: &[u8]) -> Result<(), AudioError> {
        if self.is_stopped.load(Ordering::Acquire) {
            return Err(AudioError::WriteError("Sink is stopped".to_string()));
        }

        let mut writer = self.writer.lock().await;

        // Convert bytes to i16 samples
        for chunk in audio_data.chunks(2) {
            if chunk.len() != 2 {
                return Err(AudioError::WriteError("Incomplete sample".to_string()));
            }
            let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
            writer
                .write_sample(sample)
                .map_err(|e| AudioError::WriteError(format!("Failed to write sample: {}", e)))?;
        }

        Ok(())
    }

    async fn stop(&self) -> Result<(), AudioError> {
        self.is_stopped.store(true, Ordering::Release);

        // Flush and finalize the WAV file
        let mut writer = self.writer.lock().await;
        writer
            .flush()
            .map_err(|e| AudioError::StopError(format!("Failed to flush WAV file: {}", e)))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn test_wav_file_sink() -> Result<(), Box<dyn std::error::Error>> {
        // Create a temporary file
        let temp_file = NamedTempFile::new()?;
        let temp_path = temp_file.path().to_owned();

        // Create sink and write some audio
        let sink = WavFileSink::new(&temp_path)?;

        // Generate a simple sine wave
        let sample_rate = 16000;
        let duration_ms = 100;
        let num_samples = sample_rate * duration_ms / 1000;
        let frequency = 440.0; // A4 note

        let mut samples = Vec::new();
        for i in 0..num_samples {
            let t = i as f32 / sample_rate as f32;
            let value = (2.0 * std::f32::consts::PI * frequency * t).sin();
            let sample = (value * i16::MAX as f32) as i16;
            samples.extend_from_slice(&sample.to_le_bytes());
        }

        // Write and stop
        sink.write(&samples).await?;
        sink.stop().await?;

        // Verify file exists and has content
        assert!(temp_path.exists());
        assert!(fs::metadata(&temp_path)?.len() > 0);

        // Clean up
        fs::remove_file(&temp_path)?;

        Ok(())
    }

    #[tokio::test]
    async fn test_wav_file_sink_invalid_data() {
        let temp_file = NamedTempFile::new().unwrap();
        let sink = WavFileSink::new(temp_file.path()).unwrap();

        // Try to write odd number of bytes (invalid for 16-bit samples)
        let result = sink.write(&[1, 2, 3]).await;
        assert!(result.is_err());

        if let Err(AudioError::WriteError(msg)) = result {
            assert!(msg.contains("Incomplete sample"));
        } else {
            panic!("Expected WriteError for incomplete sample");
        }
    }
}

/// Configuration for RodioSink
#[derive(Debug, Clone)]
pub struct RodioConfig {
    /// Buffer size in milliseconds (default 30000ms = 30s)
    pub buffer_size_ms: u32,
    /// Warning threshold for low buffer (percentage)
    pub low_buffer_warning: u8,
    /// Warning threshold for high buffer (percentage)
    pub high_buffer_warning: u8,
}

impl Default for RodioConfig {
    fn default() -> Self {
        Self {
            buffer_size_ms: 30_000,  // 30 seconds
            low_buffer_warning: 25,  // 25%
            high_buffer_warning: 75, // 75%
        }
    }
}

/// Statistics for monitoring RodioSink performance
#[derive(Debug)]
pub struct RodioStats {
    buffer_samples: AtomicUsize,
    max_buffer_samples: usize,
    last_write: Mutex<Instant>,
    write_interval_ms: AtomicUsize,
}

impl RodioStats {
    fn new(max_buffer_samples: usize) -> Self {
        Self {
            buffer_samples: AtomicUsize::new(0),
            max_buffer_samples,
            last_write: Mutex::new(Instant::now()),
            write_interval_ms: AtomicUsize::new(0),
        }
    }

    fn buffer_percentage(&self) -> u8 {
        let current = self.buffer_samples.load(Ordering::Acquire);
        ((current * 100) / self.max_buffer_samples) as u8
    }
}

/// A sink that plays audio in real-time using rodio
pub struct RodioSink {
    audio_sender: Sender<AudioCommand>,
    stats: Arc<RodioStats>,
    config: RodioConfig,
    is_stopped: Arc<AtomicBool>,
}

enum AudioCommand {
    PlayAudio(Vec<u8>),
    Stop,
}

impl RodioSink {
    /// Create a new RodioSink with the specified configuration
    pub fn new(config: RodioConfig) -> Result<Self, AudioError> {
        let (tx, rx) = channel();
        let stats = Arc::new(RodioStats::new(
            (16000 * config.buffer_size_ms as usize) / 1000,
        ));
        let stats_clone = Arc::clone(&stats);

        // Spawn audio playback thread
        thread::spawn(move || {
            if let Err(e) = Self::run_audio_thread(rx, stats_clone) {
                error!("Audio thread error: {}", e);
            }
        });

        Ok(Self {
            audio_sender: tx,
            stats,
            config,
            is_stopped: Arc::new(AtomicBool::new(false)),
        })
    }

    fn run_audio_thread(
        rx: Receiver<AudioCommand>,
        stats: Arc<RodioStats>,
    ) -> Result<(), AudioError> {
        // Initialize audio output
        let (_stream, stream_handle) = OutputStream::try_default()
            .map_err(|e| AudioError::WriteError(format!("Failed to open audio output: {}", e)))?;

        let sink = Sink::try_new(&stream_handle)
            .map_err(|e| AudioError::WriteError(format!("Failed to create audio sink: {}", e)))?;

        loop {
            match rx.recv() {
                Ok(AudioCommand::PlayAudio(data)) => {
                    if let Ok(source) = PCMSource::new(&data) {
                        sink.append(source);
                        stats
                            .buffer_samples
                            .fetch_add(data.len() / 2, Ordering::Release);
                    }
                }
                Ok(AudioCommand::Stop) => {
                    sink.stop();
                    break;
                }
                Err(_) => break, // Channel closed
            }
        }

        Ok(())
    }

    /// Get current buffer statistics
    pub fn get_stats(&self) -> (u8, usize) {
        (
            self.stats.buffer_percentage(),
            self.stats.write_interval_ms.load(Ordering::Acquire),
        )
    }
}

/// Convert raw PCM bytes to a rodio Source
struct PCMSource {
    samples: Vec<i16>,
    position: usize,
}

impl PCMSource {
    fn new(audio_data: &[u8]) -> Result<Self, AudioError> {
        if audio_data.len() % 2 != 0 {
            return Err(AudioError::WriteError("Incomplete sample".to_string()));
        }

        let mut samples = Vec::with_capacity(audio_data.len() / 2);
        for chunk in audio_data.chunks_exact(2) {
            samples.push(i16::from_le_bytes([chunk[0], chunk[1]]));
        }

        Ok(Self {
            samples,
            position: 0,
        })
    }
}

impl Iterator for PCMSource {
    type Item = i16;

    fn next(&mut self) -> Option<Self::Item> {
        if self.position < self.samples.len() {
            let sample = self.samples[self.position];
            self.position += 1;
            Some(sample)
        } else {
            None
        }
    }
}

impl Source for PCMSource {
    fn current_frame_len(&self) -> Option<usize> {
        Some(self.samples.len() - self.position)
    }

    fn channels(&self) -> u16 {
        1
    }

    fn sample_rate(&self) -> u32 {
        16000
    }

    fn total_duration(&self) -> Option<Duration> {
        Some(Duration::from_secs_f32(
            self.samples.len() as f32 / self.sample_rate() as f32,
        ))
    }
}

#[async_trait::async_trait]
impl AudioSink for RodioSink {
    async fn write(&self, audio_data: &[u8]) -> Result<(), AudioError> {
        if self.is_stopped.load(Ordering::Acquire) {
            return Err(AudioError::WriteError("Sink is stopped".to_string()));
        }

        // Update write interval statistics
        let now = Instant::now();
        let mut last_write = self.stats.last_write.lock().await;
        let interval = now.duration_since(*last_write).as_millis() as usize;
        self.stats
            .write_interval_ms
            .store(interval, Ordering::Release);
        *last_write = now;

        let num_samples = audio_data.len() / 2;

        // Update buffer statistics
        let current_samples = self.stats.buffer_samples.load(Ordering::Acquire);
        let buffer_percentage =
            ((current_samples + num_samples) * 100) / self.stats.max_buffer_samples;

        // Log buffer state
        match buffer_percentage {
            p if p <= self.config.low_buffer_warning as usize => {
                warn!("Audio buffer running low: {}%", p);
            }
            p if p >= self.config.high_buffer_warning as usize => {
                warn!("Audio buffer running high: {}%", p);
            }
            _ => {
                debug!("Audio buffer at {}%", buffer_percentage);
            }
        }

        if interval > 30 {
            warn!("Large gap between audio writes: {}ms", interval);
        }

        // Send audio to playback thread
        self.audio_sender
            .send(AudioCommand::PlayAudio(audio_data.to_vec()))
            .map_err(|_| AudioError::WriteError("Audio thread disconnected".to_string()))?;

        Ok(())
    }

    async fn stop(&self) -> Result<(), AudioError> {
        self.is_stopped.store(true, Ordering::Release);

        // Send stop command to audio thread
        self.audio_sender
            .send(AudioCommand::Stop)
            .map_err(|_| AudioError::StopError("Failed to stop audio thread".to_string()))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ... existing TestSink and WavFileSink tests ...

    #[tokio::test]
    async fn test_rodio_sink_basic() -> Result<(), AudioError> {
        let config = RodioConfig::default();
        let sink = RodioSink::new(config)?;

        // Generate a short sine wave
        let frequency = 440.0; // A4 note
        let duration_ms = 100;
        let num_samples = 16000 * duration_ms / 1000;
        let mut samples = Vec::new();

        for i in 0..num_samples {
            let t = i as f32 / 16000.0;
            let value = (2.0 * std::f32::consts::PI * frequency * t).sin();
            let sample = (value * i16::MAX as f32) as i16;
            samples.extend_from_slice(&sample.to_le_bytes());
        }

        // Write audio data
        sink.write(&samples).await?;

        // Check buffer statistics
        let (buffer_percent, write_interval) = sink.get_stats();
        assert!(buffer_percent > 0);
        assert!(write_interval >= 0);

        // Stop playback
        sink.stop().await?;
        assert!(sink.is_stopped.load(Ordering::Acquire));

        Ok(())
    }

    #[tokio::test]
    async fn test_rodio_sink_buffer_monitoring() -> Result<(), AudioError> {
        // Create sink with small buffer for testing
        let config = RodioConfig {
            buffer_size_ms: 1000, // 1 second buffer
            low_buffer_warning: 25,
            high_buffer_warning: 75,
        };
        let sink = RodioSink::new(config)?;

        // Generate 500ms of audio
        let num_samples = 16000 * 500 / 1000;
        let mut samples = Vec::new();
        for i in 0..num_samples {
            let sample = (i as i16 % 100) as i16;
            samples.extend_from_slice(&sample.to_le_bytes());
        }

        // Write audio and check buffer percentage
        sink.write(&samples).await?;
        let (buffer_percent, _) = sink.get_stats();
        assert!(
            buffer_percent >= 45 && buffer_percent <= 55,
            "Expected ~50% buffer usage, got {}%",
            buffer_percent
        );

        Ok(())
    }
}
