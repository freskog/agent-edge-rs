use super::{
    AudioCapture, AudioCaptureConfig, AudioCaptureError, AudioCaptureStats, AudioDeviceInfo,
};
use libpulse_binding as pulse;
use libpulse_simple_binding as psimple;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// PulseAudio implementation of AudioCapture trait
pub struct PulseAudioCapture {
    config: AudioCaptureConfig,
    simple: Option<psimple::Simple>,
    buffer: Arc<Mutex<VecDeque<i16>>>,
    is_active: bool,
}

impl AudioCapture for PulseAudioCapture {
    fn new(config: AudioCaptureConfig) -> Result<Self, AudioCaptureError> {
        let buffer = Arc::new(Mutex::new(VecDeque::new()));

        Ok(Self {
            config,
            simple: None,
            buffer,
            is_active: false,
        })
    }

    fn start(&mut self) -> Result<(), AudioCaptureError> {
        if self.is_active {
            return Err(AudioCaptureError::AlreadyStarted);
        }

        // Create sample specification - use S16LE for compatibility
        let sample_spec = pulse::sample::Spec {
            format: pulse::sample::Format::S16le,
            channels: self.config.channels,
            rate: self.config.sample_rate,
        };

        if !sample_spec.is_valid() {
            return Err(AudioCaptureError::Config(
                "Invalid sample specification".to_string(),
            ));
        }

        // Calculate fragment size based on target latency
        let bytes_per_sample = 2; // i16 = 2 bytes
        let samples_per_ms = self.config.sample_rate / 1000;
        let samples_for_latency = samples_per_ms * self.config.target_latency_ms;
        let fragsize = samples_for_latency * self.config.channels as u32 * bytes_per_sample;

        let buffer_attr = pulse::def::BufferAttr {
            maxlength: std::u32::MAX,
            tlength: std::u32::MAX,
            prebuf: std::u32::MAX,
            minreq: std::u32::MAX,
            fragsize,
        };

        // Create PulseAudio simple connection
        let simple = psimple::Simple::new(
            None,
            &self.config.app_name,
            pulse::stream::Direction::Record,
            self.config.device_name.as_deref(),
            &self.config.stream_name,
            &sample_spec,
            None,
            Some(&buffer_attr),
        )
        .map_err(|e| {
            AudioCaptureError::Device(format!("Failed to create PulseAudio connection: {}", e))
        })?;

        self.simple = Some(simple);
        self.is_active = true;

        log::info!(
            "PulseAudio: Started capture - {} channels, {} Hz, target latency: {}ms",
            self.config.channels,
            self.config.sample_rate,
            self.config.target_latency_ms
        );

        Ok(())
    }

    fn stop(&mut self) -> Result<(), AudioCaptureError> {
        if !self.is_active {
            return Ok(());
        }

        self.simple = None;
        self.is_active = false;

        // Clear any buffered data
        self.buffer.lock().unwrap().clear();

        log::info!("PulseAudio: Stopped capture");
        Ok(())
    }

    fn read_chunk(&mut self) -> Result<Vec<i16>, AudioCaptureError> {
        if !self.is_active {
            return Err(AudioCaptureError::NotStarted);
        }

        let mut buffer = self.buffer.lock().unwrap();

        // Check if we have enough samples for a complete chunk (1280 samples)
        const TARGET_CHUNK_SIZE: usize = 1280;
        if buffer.len() >= TARGET_CHUNK_SIZE {
            let samples: Vec<i16> = buffer.drain(..TARGET_CHUNK_SIZE).collect();
            return Ok(samples);
        }

        // We need more data, read from PulseAudio
        drop(buffer); // Release lock before blocking read

        if let Some(ref simple) = self.simple {
            // Calculate how much data to read (aim for 1-2 chunks worth)
            let bytes_to_read = TARGET_CHUNK_SIZE * self.config.channels as usize * 2; // 2 bytes per i16
            let mut raw_buffer = vec![0u8; bytes_to_read];

            simple
                .read(&mut raw_buffer)
                .map_err(|e| AudioCaptureError::Stream(format!("PulseAudio read error: {}", e)))?;

            // Convert raw bytes to i16 samples
            let mut i16_samples = Vec::new();
            for chunk in raw_buffer.chunks_exact(2) {
                let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
                i16_samples.push(sample);
            }

            // Extract target channel if multi-channel
            let channel_samples = if self.config.channels == 1 {
                i16_samples
            } else {
                let channels = self.config.channels as usize;
                let target_channel = self.config.target_channel as usize;

                i16_samples
                    .chunks(channels)
                    .filter_map(|chunk| chunk.get(target_channel).copied())
                    .collect()
            };

            // Add to buffer
            let mut buffer = self.buffer.lock().unwrap();
            buffer.extend(channel_samples);

            // Return chunk if we have enough data
            if buffer.len() >= TARGET_CHUNK_SIZE {
                let samples: Vec<i16> = buffer.drain(..TARGET_CHUNK_SIZE).collect();
                Ok(samples)
            } else {
                Err(AudioCaptureError::NoData)
            }
        } else {
            Err(AudioCaptureError::NotStarted)
        }
    }

    fn is_active(&self) -> bool {
        self.is_active
    }

    fn available_samples(&self) -> usize {
        self.buffer.lock().unwrap().len()
    }

    fn config(&self) -> &AudioCaptureConfig {
        &self.config
    }

    async fn record_for_duration(
        &mut self,
        duration_secs: f32,
    ) -> Result<Vec<i16>, AudioCaptureError> {
        let expected_samples = (self.config.sample_rate as f32 * duration_secs) as usize;
        let mut all_samples = Vec::with_capacity(expected_samples);

        let was_active = self.is_active();
        if !was_active {
            self.start()?;
        }

        let start_time = std::time::Instant::now();
        let duration = std::time::Duration::from_secs_f32(duration_secs);

        while start_time.elapsed() < duration {
            match self.read_chunk() {
                Ok(chunk) => all_samples.extend(chunk),
                Err(AudioCaptureError::NoData) => {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
                Err(e) => return Err(e),
            }
        }

        if !was_active {
            self.stop()?;
        }

        log::info!(
            "PulseAudio: Recorded {} samples in {:.1}s",
            all_samples.len(),
            duration_secs
        );
        Ok(all_samples)
    }

    fn get_stats(&self) -> AudioCaptureStats {
        AudioCaptureStats {
            total_samples_captured: 0,
            current_sample_rate: self.config.sample_rate,
            current_channels: self.config.channels as u32,
            buffer_underruns: 0,
            buffer_overruns: 0,
        }
    }

    fn list_devices(&self) -> Result<Vec<AudioDeviceInfo>, AudioCaptureError> {
        let default_device = AudioDeviceInfo {
            name: "PulseAudio Default Device".to_string(),
            id: "default".to_string(),
            is_default: true,
            max_channels: 8,
            supported_sample_rates: vec![16000, 44100, 48000],
        };
        Ok(vec![default_device])
    }
}
