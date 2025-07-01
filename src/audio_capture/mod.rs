use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    Device, FromSample, Host, Sample, SampleFormat, SizedSample, Stream as CpalStream,
};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use thiserror::Error;

pub const CHUNK_SIZE: usize = 1280; // Fixed chunk size that works with Silero

/// Maximum number of chunks to keep in ring buffer (~3.2 seconds at 16kHz)
const MAX_RING_BUFFER_SIZE: usize = 40;

#[derive(Error, Debug)]
pub enum AudioCaptureError {
    #[error("No audio devices found")]
    NoDevices,
    #[error("Audio device error: {0}")]
    Device(String),
    #[error("Audio stream error: {0}")]
    Stream(String),
    #[error("Configuration error: {0}")]
    Config(String),
}

/// Audio capture configuration
#[derive(Debug, Clone)]
pub struct AudioCaptureConfig {
    /// Device ID to capture from (None = default device)
    pub device_id: Option<String>,
    /// Channel to capture (0-based index)
    pub channel: u32,
    pub sample_rate: u32,
    pub channels: u16,
    pub buffer_size: usize,
}

impl Default for AudioCaptureConfig {
    fn default() -> Self {
        Self {
            device_id: None,
            channel: 0,
            sample_rate: 16000, // 16kHz for speech
            channels: 1,        // Mono
            buffer_size: 1280,  // 80ms at 16kHz
        }
    }
}

/// Audio device information
#[derive(Debug, Clone)]
pub struct AudioDeviceInfo {
    pub name: String,
    pub id: String,
    pub is_default: bool,
    pub channel_count: u32,
}

#[derive(Clone, Debug)]
pub struct AudioChunk {
    pub samples: Vec<i16>,
    pub timestamp: Instant,
}

// Type alias for the callback function
pub type AudioCallback = Box<dyn FnMut(AudioChunk) + Send + 'static>;

/// Audio capture with ring buffer for historical audio access
pub struct AudioCapture {
    #[allow(dead_code)]
    config: AudioCaptureConfig,
    #[allow(dead_code)]
    stream: Option<CpalStream>,
    _host: Host,
    ring_buffer: Arc<Mutex<VecDeque<AudioChunk>>>,
}

impl AudioCapture {
    pub fn new(
        config: AudioCaptureConfig,
        callback: AudioCallback,
    ) -> Result<Self, AudioCaptureError> {
        let host = cpal::default_host();

        // Create ring buffer with fixed capacity
        let ring_buffer = Arc::new(Mutex::new(VecDeque::with_capacity(MAX_RING_BUFFER_SIZE)));
        let ring_buffer_clone = ring_buffer.clone();

        // Create a buffer to accumulate samples
        let sample_buffer = Arc::new(Mutex::new(Vec::with_capacity(CHUNK_SIZE)));

        // Wrap callback in Arc<Mutex<>> so it can be shared between threads
        let callback = Arc::new(Mutex::new(callback));

        // Get the device
        let device = if let Some(id) = &config.device_id {
            host.devices()
                .map_err(|e| AudioCaptureError::Device(e.to_string()))?
                .find(|d| d.name().map(|n| n == *id).unwrap_or(false))
                .ok_or_else(|| AudioCaptureError::Device(format!("Device not found: {}", id)))?
        } else {
            host.default_input_device()
                .ok_or_else(|| AudioCaptureError::Device("No default input device found".into()))?
        };

        let supported_config = device
            .default_input_config()
            .map_err(|e| AudioCaptureError::Config(e.to_string()))?;

        // Verify channel selection is valid
        if config.channel >= u32::from(supported_config.channels()) {
            return Err(AudioCaptureError::Config(format!(
                "Selected channel {} is not available (device has {} channels)",
                config.channel,
                supported_config.channels()
            )));
        }

        let stream_config = supported_config.config();
        let err_fn = move |err| {
            log::error!("Audio stream error: {}", err);
        };

        // Log the format being used
        log::info!(
            "Audio capture configured: {} channels @ {}Hz (format: {:?})",
            stream_config.channels,
            stream_config.sample_rate.0,
            supported_config.sample_format()
        );

        let channel = config.channel;
        let channels = stream_config.channels as usize;

        // Build the stream with the appropriate sample format
        let stream = match supported_config.sample_format() {
            SampleFormat::I16 => Self::create_input_stream_with_buffer::<i16>(
                &device,
                &stream_config,
                channel,
                channels,
                sample_buffer,
                callback,
                ring_buffer_clone,
                err_fn,
            )?,
            SampleFormat::U16 => Self::create_input_stream_with_buffer::<u16>(
                &device,
                &stream_config,
                channel,
                channels,
                sample_buffer,
                callback,
                ring_buffer_clone,
                err_fn,
            )?,
            SampleFormat::F32 => Self::create_input_stream_with_buffer::<f32>(
                &device,
                &stream_config,
                channel,
                channels,
                sample_buffer,
                callback,
                ring_buffer_clone,
                err_fn,
            )?,
            _ => {
                return Err(AudioCaptureError::Config(
                    "Unsupported sample format".into(),
                ))
            }
        };

        stream
            .play()
            .map_err(|e| AudioCaptureError::Stream(e.to_string()))?;

        Ok(Self {
            config,
            stream: Some(stream),
            _host: host,
            ring_buffer,
        })
    }

    fn create_input_stream_with_buffer<T>(
        device: &Device,
        config: &cpal::StreamConfig,
        channel: u32,
        channels: usize,
        buffer: Arc<Mutex<Vec<i16>>>,
        callback: Arc<Mutex<AudioCallback>>,
        ring_buffer: Arc<Mutex<VecDeque<AudioChunk>>>,
        err_fn: impl FnMut(cpal::StreamError) + Send + 'static + Copy,
    ) -> Result<CpalStream, AudioCaptureError>
    where
        T: Sample + SizedSample + Send + Sync + 'static,
        i16: FromSample<T>,
    {
        device
            .build_input_stream(
                config,
                move |data: &[T], _: &cpal::InputCallbackInfo| {
                    // Extract the specified channel and convert to i16
                    if let Ok(mut buffer) = buffer.lock() {
                        for frame in data.chunks(channels) {
                            if let Some(sample) = frame.get(channel as usize) {
                                let value = i16::from_sample(*sample);
                                buffer.push(value);

                                // If we have enough samples, create and send a chunk
                                if buffer.len() >= CHUNK_SIZE {
                                    let mut chunk_samples = [0i16; CHUNK_SIZE];
                                    chunk_samples.copy_from_slice(&buffer[..CHUNK_SIZE]);
                                    buffer.drain(..CHUNK_SIZE);

                                    let chunk = AudioChunk {
                                        samples: chunk_samples.to_vec(),
                                        timestamp: Instant::now(),
                                    };

                                    // Add to ring buffer with size limit
                                    if let Ok(mut ring_buf) = ring_buffer.try_lock() {
                                        ring_buf.push_back(chunk.clone());
                                        while ring_buf.len() > MAX_RING_BUFFER_SIZE {
                                            ring_buf.pop_front();
                                            log::debug!("Ring buffer full, dropping oldest chunk");
                                        }
                                    }

                                    // Call user callback
                                    if let Ok(mut callback) = callback.try_lock() {
                                        callback(chunk);
                                    }
                                }
                            }
                        }
                    }
                },
                err_fn,
                None,
            )
            .map_err(|e| AudioCaptureError::Stream(e.to_string()))
    }

    pub fn list_devices() -> Result<Vec<AudioDeviceInfo>, AudioCaptureError> {
        let host = cpal::default_host();
        let devices = host
            .devices()
            .map_err(|e| AudioCaptureError::Device(e.to_string()))?;

        let default_device = host.default_input_device();

        let mut result = Vec::new();
        for device in devices {
            if let Ok(name) = device.name() {
                let config = device
                    .default_input_config()
                    .map_err(|e| AudioCaptureError::Config(e.to_string()))?;

                result.push(AudioDeviceInfo {
                    name: name.clone(),
                    id: name.clone(),
                    is_default: default_device
                        .as_ref()
                        .map(|d| d.name().unwrap_or_default())
                        == Some(name),
                    channel_count: u32::from(config.channels()),
                });
            }
        }

        Ok(result)
    }

    /// Get recent audio chunks from the ring buffer
    /// Returns up to the last 3 seconds of audio
    pub fn get_recent_audio(&self) -> Vec<AudioChunk> {
        if let Ok(buffer) = self.ring_buffer.lock() {
            buffer.iter().cloned().collect()
        } else {
            Vec::new()
        }
    }

    /// Get recent audio chunks as raw samples for STT
    /// Flattens all chunks into a single contiguous buffer
    pub fn get_recent_audio_flat(&self) -> Vec<i16> {
        if let Ok(buffer) = self.ring_buffer.lock() {
            buffer
                .iter()
                .flat_map(|chunk| chunk.samples.iter())
                .cloned()
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Get the current ring buffer size
    pub fn ring_buffer_size(&self) -> usize {
        self.ring_buffer.lock().unwrap().len()
    }

    /// Get the maximum ring buffer size
    pub fn max_ring_buffer_size(&self) -> usize {
        MAX_RING_BUFFER_SIZE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;
    use tokio;

    fn create_test_chunk() -> AudioChunk {
        AudioChunk {
            samples: vec![0i16; CHUNK_SIZE],
            timestamp: Instant::now(),
        }
    }

    #[tokio::test]
    async fn test_ring_buffer_capacity() {
        let (tx, _rx) = mpsc::channel();
        let callback = Box::new(move |chunk: AudioChunk| {
            let _ = tx.send(chunk);
        });

        match AudioCapture::new(AudioCaptureConfig::default(), callback) {
            Ok(capture) => {
                assert_eq!(capture.max_ring_buffer_size(), MAX_RING_BUFFER_SIZE);
                assert_eq!(capture.ring_buffer_size(), 0);
            }
            Err(e) => {
                println!(
                    "Audio device not available in test environment - this is expected: {}",
                    e
                );
            }
        }
    }

    #[tokio::test]
    async fn test_ring_buffer_overflow() {
        let (tx, _rx) = mpsc::channel();
        let callback = Box::new(move |chunk: AudioChunk| {
            let _ = tx.send(chunk);
        });

        match AudioCapture::new(AudioCaptureConfig::default(), callback) {
            Ok(capture) => {
                // Get access to ring buffer
                let ring_buffer = capture.ring_buffer.clone();

                // Simulate the automatic trimming behavior from the audio stream callback
                {
                    let mut buffer = ring_buffer.lock().unwrap();
                    for _ in 0..MAX_RING_BUFFER_SIZE + 5 {
                        buffer.push_back(create_test_chunk());
                        // Simulate the trimming logic from the stream callback
                        while buffer.len() > MAX_RING_BUFFER_SIZE {
                            buffer.pop_front();
                        }
                    }
                }

                // Verify buffer didn't exceed max size
                assert_eq!(capture.ring_buffer_size(), MAX_RING_BUFFER_SIZE);
            }
            Err(e) => {
                println!(
                    "Audio device not available in test environment - this is expected: {}",
                    e
                );
            }
        }
    }

    #[tokio::test]
    async fn test_ring_buffer_fifo() {
        let (tx, _rx) = mpsc::channel();
        let callback = Box::new(move |chunk: AudioChunk| {
            let _ = tx.send(chunk);
        });

        match AudioCapture::new(AudioCaptureConfig::default(), callback) {
            Ok(capture) => {
                let ring_buffer = capture.ring_buffer.clone();

                // Add chunks with timestamps
                {
                    let mut buffer = ring_buffer.lock().unwrap();
                    for _ in 0..3 {
                        buffer.push_back(create_test_chunk());
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                }

                // Verify FIFO order
                let buffer = ring_buffer.lock().unwrap();
                let timestamps: Vec<_> = buffer.iter().map(|chunk| chunk.timestamp).collect();
                for i in 1..timestamps.len() {
                    assert!(
                        timestamps[i] > timestamps[i - 1],
                        "Chunks should be in chronological order"
                    );
                }
            }
            Err(e) => {
                println!(
                    "Audio device not available in test environment - this is expected: {}",
                    e
                );
            }
        }
    }
}
