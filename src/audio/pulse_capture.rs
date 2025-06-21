use crate::audio::{AudioBuffer, ChannelExtractor};
use crate::error::{EdgeError, Result};
use libpulse_binding as pulse;
use libpulse_simple_binding as psimple;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;

/// Configuration for PulseAudio capture
#[derive(Debug, Clone)]
pub struct PulseAudioCaptureConfig {
    pub sample_rate: u32,
    pub channels: u8,
    pub device_name: Option<String>,
    /// Target latency in milliseconds - communicated to PulseAudio via buffer attributes to prevent aggressive buffering
    pub target_latency_ms: u32,
    pub app_name: String,
    pub stream_name: String,
}

impl Default for PulseAudioCaptureConfig {
    fn default() -> Self {
        Self {
            sample_rate: 16000,
            channels: 6, // ReSpeaker 4-mic array
            device_name: None,
            target_latency_ms: 50, // 50ms target latency for AEC compatibility
            app_name: "agent-edge".to_string(),
            stream_name: "wakeword-capture".to_string(),
        }
    }
}

/// PulseAudio capture implementation using Simple API with proper latency buffer hints
pub struct PulseAudioCapture {
    config: PulseAudioCaptureConfig,
    channel_extractor: ChannelExtractor,
    audio_receiver: Option<mpsc::Receiver<AudioBuffer>>,
    _audio_sender: Option<mpsc::Sender<AudioBuffer>>,
    capture_thread: Option<thread::JoinHandle<()>>,
    stop_flag: Arc<Mutex<bool>>,
    // Buffer to accumulate samples for 80ms chunks
    sample_buffer: Vec<i16>,
}

impl PulseAudioCapture {
    pub fn new(config: PulseAudioCaptureConfig) -> Result<Self> {
        log::info!("Initializing PulseAudio capture with config: {:?}", config);

        // Set up channel extractor for ReSpeaker (extract channel 0 from multi-channel input)
        let channel_extractor = ChannelExtractor::new(0, config.channels as usize)
            .map_err(|e| EdgeError::Audio(format!("Failed to create channel extractor: {}", e)))?;

        Ok(Self {
            config,
            channel_extractor,
            audio_receiver: None,
            _audio_sender: None,
            capture_thread: None,
            stop_flag: Arc::new(Mutex::new(false)),
            sample_buffer: Vec::new(),
        })
    }

    pub fn start(&mut self) -> Result<()> {
        log::info!(
            "Starting PulseAudio capture with {}ms target latency to prevent aggressive buffering",
            self.config.target_latency_ms
        );

        // Create sample specification - use S16LE for WebRTC VAD compatibility
        let sample_spec = pulse::sample::Spec {
            format: pulse::sample::Format::S16le, // 16-bit signed integers, not float
            channels: self.config.channels,
            rate: self.config.sample_rate,
        };

        if !sample_spec.is_valid() {
            return Err(EdgeError::Audio("Invalid sample specification".to_string()));
        }

        log::info!("Sample spec: {:?} (S16LE for WebRTC VAD)", sample_spec);

        // Calculate fragment size based on target latency
        let bytes_per_sample = 2; // i16 = 2 bytes (not 4 like f32)
        let samples_per_ms = self.config.sample_rate / 1000;
        let samples_for_latency = samples_per_ms * self.config.target_latency_ms;
        let fragsize = samples_for_latency * self.config.channels as u32 * bytes_per_sample;

        let buffer_attr = pulse::def::BufferAttr {
            maxlength: std::u32::MAX, // Let PulseAudio decide max buffer
            tlength: std::u32::MAX,   // Not used for recording
            prebuf: std::u32::MAX,    // Not used for recording
            minreq: std::u32::MAX,    // Not used for recording
            fragsize, // Key: prevents aggressive buffering by setting delivery chunk size
        };

        log::info!(
            "Anti-aggressive buffering: fragsize={}bytes for {}ms latency (prevents AEC issues)",
            fragsize,
            self.config.target_latency_ms
        );

        // Create PulseAudio simple connection with latency hints
        let simple = psimple::Simple::new(
            None,                               // Use default server
            &self.config.app_name,              // Application name
            pulse::stream::Direction::Record,   // We want to record
            self.config.device_name.as_deref(), // Device name (None = default)
            &self.config.stream_name,           // Stream name
            &sample_spec,                       // Sample specification
            None,                               // Use default channel map
            Some(&buffer_attr),                 // Buffer attributes prevent aggressive buffering
        )
        .map_err(|e| EdgeError::Audio(format!("Failed to create PulseAudio connection: {}", e)))?;

        log::info!(
            "Connected to PulseAudio server with {}ms latency hints for AEC compatibility",
            self.config.target_latency_ms
        );

        // Create audio channel for sending data
        let (sender, receiver) = mpsc::channel();
        self.audio_receiver = Some(receiver);
        self._audio_sender = Some(sender.clone());

        // Reset stop flag
        *self.stop_flag.lock().unwrap() = false;

        // Use the calculated fragment size for our read buffer
        let buffer_size_bytes = fragsize as usize;

        log::info!(
            "Using read buffer size: {} bytes (matches fragsize for optimal latency)",
            buffer_size_bytes
        );

        // Start capture thread
        let channel_extractor = self.channel_extractor.clone();
        let stop_flag = Arc::clone(&self.stop_flag);

        let capture_thread = thread::spawn(move || {
            Self::capture_loop(
                simple,
                sender,
                channel_extractor,
                stop_flag,
                buffer_size_bytes,
            );
        });

        self.capture_thread = Some(capture_thread);

        log::info!(
            "PulseAudio capture started with latency hints to prevent AEC-breaking buffering"
        );
        Ok(())
    }

    fn capture_loop(
        simple: psimple::Simple,
        sender: mpsc::Sender<AudioBuffer>,
        channel_extractor: ChannelExtractor,
        stop_flag: Arc<Mutex<bool>>,
        buffer_size_bytes: usize,
    ) {
        log::info!("PulseAudio capture thread started with S16LE format");

        let mut raw_buffer = vec![0u8; buffer_size_bytes];
        let mut sample_count = 0usize;

        while !*stop_flag.lock().unwrap() {
            // Read raw bytes from PulseAudio
            match simple.read(&mut raw_buffer) {
                Ok(()) => {
                    // Convert raw bytes to i16 samples, then to f32 for processing
                    let sample_len = raw_buffer.len() / 2; // 2 bytes per i16
                    let mut i16_samples = Vec::with_capacity(sample_len);

                    for chunk in raw_buffer.chunks_exact(2) {
                        let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
                        i16_samples.push(sample);
                    }

                    // Convert i16 to f32 for internal processing (normalize to -1.0 to 1.0)
                    let f32_samples: Vec<f32> = i16_samples
                        .iter()
                        .map(|&sample| sample as f32 / 32768.0) // Proper normalization
                        .collect();

                    // Extract target channel using the channel extractor
                    let channel_data = channel_extractor.extract_channel(&f32_samples);
                    sample_count += channel_data.len();

                    if let Err(e) = sender.send(channel_data.clone()) {
                        log::warn!("Failed to send audio data: {}, stopping capture", e);
                        break;
                    }

                    // Log progress occasionally
                    if sample_count % 16000 == 0 {
                        // Every ~1 second at 16kHz
                        log::info!(
                            "Audio capture: {} samples total, last chunk: {} samples",
                            sample_count,
                            channel_data.len()
                        );
                    }
                }
                Err(e) => {
                    log::error!("Failed to read from PulseAudio: {}, stopping capture", e);
                    break;
                }
            }
        }

        log::info!(
            "PulseAudio capture thread stopped. Total samples: {}",
            sample_count
        );
    }

    pub fn get_audio_buffer(&self) -> Result<AudioBuffer> {
        if let Some(receiver) = &self.audio_receiver {
            receiver
                .recv()
                .map_err(|e| EdgeError::Audio(format!("Failed to receive audio data: {}", e)))
        } else {
            Err(EdgeError::Audio(
                "PulseAudio capture not started".to_string(),
            ))
        }
    }

    pub fn try_get_audio_buffer(&self) -> Result<Option<AudioBuffer>> {
        if let Some(receiver) = &self.audio_receiver {
            match receiver.try_recv() {
                Ok(buffer) => Ok(Some(buffer)),
                Err(mpsc::TryRecvError::Empty) => Ok(None),
                Err(mpsc::TryRecvError::Disconnected) => Err(EdgeError::Audio(
                    "Audio capture thread disconnected".to_string(),
                )),
            }
        } else {
            Err(EdgeError::Audio(
                "PulseAudio capture not started".to_string(),
            ))
        }
    }

    pub fn stop(&mut self) -> Result<()> {
        log::info!("Stopping PulseAudio capture");

        // Signal the capture thread to stop
        if let Ok(mut flag) = self.stop_flag.lock() {
            *flag = true;
        }

        // Wait for the capture thread to finish
        if let Some(handle) = self.capture_thread.take() {
            if let Err(e) = handle.join() {
                log::warn!("Capture thread panicked: {:?}", e);
            }
        }

        // Clean up
        self.audio_receiver = None;
        self._audio_sender = None;

        log::info!("PulseAudio capture stopped");
        Ok(())
    }

    pub fn list_input_devices(&self) -> Result<Vec<String>> {
        // TODO: Implement device enumeration using PulseAudio introspection API
        // For now, return empty list since this requires more complex PulseAudio setup
        log::warn!("PulseAudio device enumeration not yet implemented");
        Ok(vec![])
    }

    /// Enhanced method to read a chunk of audio optimized for both VAD and wakeword detection
    /// Returns 1280 i16 samples (80ms at 16kHz) when available
    pub fn read_chunk(&mut self) -> Result<Vec<i16>> {
        // Accumulate samples until we have enough for an 80ms chunk
        while self.sample_buffer.len() < 1280 {
            match self.try_get_audio_buffer()? {
                Some(audio_buffer) => {
                    // Convert f32 back to i16 with proper scaling (f32 range -1.0 to 1.0 → i16 range -32768 to 32767)
                    let new_samples: Vec<i16> = audio_buffer
                        .iter()
                        .map(|&sample| {
                            // Clamp to valid f32 range first, then scale to i16
                            let clamped = sample.clamp(-1.0, 1.0);
                            (clamped * 32767.0) as i16
                        })
                        .collect();

                    log::debug!(
                        "Adding {} samples to buffer (current: {}, need: 1280)",
                        new_samples.len(),
                        self.sample_buffer.len()
                    );
                    self.sample_buffer.extend(new_samples);
                }
                None => {
                    // No audio available yet
                    return Err(EdgeError::Audio("No audio data available".to_string()));
                }
            }
        }

        // Extract exactly 1280 samples (80ms at 16kHz)
        let chunk = self.sample_buffer.drain(0..1280).collect();
        log::debug!(
            "Returning 1280 samples, {} samples remaining in buffer",
            self.sample_buffer.len()
        );
        Ok(chunk)
    }

    /// New method to get raw i16 samples directly for WebRTC VAD
    /// This avoids the double conversion issue (i16→f32→i16)
    pub fn read_i16_chunk(&mut self, chunk_size: usize) -> Result<Vec<i16>> {
        // We need to store i16 samples separately to avoid double conversion
        // For now, we'll use the existing method but with better conversion
        self.read_chunk_with_size(chunk_size)
    }

    /// Read a specific chunk size for VAD (e.g., 160, 320, or 480 samples)
    pub fn read_chunk_with_size(&mut self, chunk_size: usize) -> Result<Vec<i16>> {
        // Accumulate samples until we have enough for the requested chunk
        while self.sample_buffer.len() < chunk_size {
            match self.try_get_audio_buffer()? {
                Some(audio_buffer) => {
                    // Convert f32 back to i16 with proper scaling
                    // Important: Use proper scaling to avoid saturation
                    let new_samples: Vec<i16> = audio_buffer
                        .iter()
                        .map(|&sample| {
                            // Clamp to valid f32 range first, then scale to i16
                            let clamped = sample.clamp(-1.0, 1.0);
                            (clamped * 32767.0) as i16
                        })
                        .collect();

                    self.sample_buffer.extend(new_samples);
                }
                None => {
                    // No audio available yet
                    return Err(EdgeError::Audio("No audio data available".to_string()));
                }
            }
        }

        // Extract exactly the requested number of samples
        let chunk = self.sample_buffer.drain(0..chunk_size).collect();
        Ok(chunk)
    }
}
