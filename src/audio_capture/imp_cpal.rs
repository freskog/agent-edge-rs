use super::{
    AudioCapture, AudioCaptureConfig, AudioCaptureError, AudioCaptureStats, AudioDeviceInfo,
};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, Host, Sample, SampleFormat, Stream, StreamConfig, SupportedStreamConfig};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// CPAL implementation of AudioCapture trait
pub struct CpalAudioCapture {
    config: AudioCaptureConfig,
    _host: Host,
    device: Device,
    stream_config: StreamConfig,
    sample_format: SampleFormat,
    stream: Option<Stream>,
    buffer: Arc<Mutex<VecDeque<i16>>>,
    is_active: Arc<Mutex<bool>>,
    stats: Arc<Mutex<AudioCaptureStats>>,
    resample_ratio: f32,
    resample_buffer: Arc<Mutex<Vec<f32>>>,
}

impl AudioCapture for CpalAudioCapture {
    fn new(config: AudioCaptureConfig) -> Result<Self, AudioCaptureError> {
        let host = cpal::default_host();

        let device = if let Some(ref device_name) = config.device_name {
            host.input_devices()
                .map_err(|e| {
                    AudioCaptureError::Device(format!("Failed to enumerate devices: {}", e))
                })?
                .find(|dev| dev.name().unwrap_or_default() == *device_name)
                .ok_or_else(|| {
                    AudioCaptureError::Device(format!("Device '{}' not found", device_name))
                })?
        } else {
            host.default_input_device().ok_or_else(|| {
                AudioCaptureError::Device("No default input device available".to_string())
            })?
        };

        let (stream_config, sample_format) = Self::find_best_config(&device, &config)?;
        let buffer = Arc::new(Mutex::new(VecDeque::new()));
        let is_active = Arc::new(Mutex::new(false));
        let stats = Arc::new(Mutex::new(AudioCaptureStats {
            total_samples_captured: 0,
            current_sample_rate: stream_config.sample_rate.0,
            current_channels: stream_config.channels as u32,
            buffer_underruns: 0,
            buffer_overruns: 0,
        }));

        let actual_sample_rate = stream_config.sample_rate.0;
        let resample_ratio = actual_sample_rate as f32 / config.sample_rate as f32;
        let resample_buffer = Arc::new(Mutex::new(Vec::new()));

        Ok(Self {
            config,
            _host: host,
            device,
            stream_config,
            sample_format,
            stream: None,
            buffer,
            is_active,
            stats,
            resample_ratio,
            resample_buffer,
        })
    }

    fn start(&mut self) -> Result<(), AudioCaptureError> {
        if self.stream.is_some() {
            return Err(AudioCaptureError::AlreadyStarted);
        }

        let stream = match self.sample_format {
            SampleFormat::I16 => self.build_stream::<i16>()?,
            SampleFormat::F32 => self.build_stream::<f32>()?,
            format => {
                return Err(AudioCaptureError::Config(format!(
                    "Unsupported format: {:?}",
                    format
                )));
            }
        };

        stream
            .play()
            .map_err(|e| AudioCaptureError::Stream(format!("Failed to start stream: {}", e)))?;

        *self.is_active.lock().unwrap() = true;
        self.stream = Some(stream);
        Ok(())
    }

    fn stop(&mut self) -> Result<(), AudioCaptureError> {
        if let Some(stream) = self.stream.take() {
            drop(stream);
            *self.is_active.lock().unwrap() = false;
        }
        Ok(())
    }

    fn read_chunk(&mut self) -> Result<Vec<i16>, AudioCaptureError> {
        if !self.is_active() {
            return Err(AudioCaptureError::NotStarted);
        }

        let mut buffer = self.buffer.lock().unwrap();

        // Check if we have enough samples for a complete chunk (1280 samples)
        const TARGET_CHUNK_SIZE: usize = 1280;

        log::trace!(
            "CPAL read_chunk: buffer has {} samples, need {}",
            buffer.len(),
            TARGET_CHUNK_SIZE
        );

        if buffer.len() < TARGET_CHUNK_SIZE {
            return Err(AudioCaptureError::NoData);
        }

        // Extract exactly TARGET_CHUNK_SIZE samples
        let samples: Vec<i16> = buffer.drain(..TARGET_CHUNK_SIZE).collect();
        log::debug!(
            "CPAL: Delivered chunk of {} samples, {} remaining in buffer",
            samples.len(),
            buffer.len()
        );
        Ok(samples)
    }

    fn is_active(&self) -> bool {
        *self.is_active.lock().unwrap()
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
        // Clear buffer and start recording
        self.buffer.lock().unwrap().clear();
        let was_active = self.is_active();
        if !was_active {
            self.start()?;
        }

        // Clone the buffer Arc so we don't hold &mut self across await
        let buffer_clone = Arc::clone(&self.buffer);

        // Sleep without holding &mut self
        let duration = std::time::Duration::from_secs_f32(duration_secs);
        tokio::time::sleep(duration).await;

        // Stop if we started it
        if !was_active {
            self.stop()?;
        }

        // Collect samples from cloned buffer
        let mut buffer = buffer_clone.lock().unwrap();
        let samples: Vec<i16> = buffer.drain(..).collect();

        log::info!(
            "CPAL: Recorded {} samples in {:.1}s",
            samples.len(),
            duration_secs
        );
        Ok(samples)
    }

    fn get_stats(&self) -> AudioCaptureStats {
        self.stats.lock().unwrap().clone()
    }

    fn list_devices(&self) -> Result<Vec<AudioDeviceInfo>, AudioCaptureError> {
        let host = cpal::default_host();
        let devices = host.input_devices().map_err(|e| {
            AudioCaptureError::Device(format!("Failed to enumerate devices: {}", e))
        })?;

        let mut device_infos = Vec::new();
        for device in devices {
            let name = device.name().unwrap_or("Unknown Device".to_string());
            device_infos.push(AudioDeviceInfo {
                name: name.clone(),
                id: name,
                is_default: false,
                max_channels: 2,
                supported_sample_rates: vec![16000, 44100, 48000],
            });
        }

        Ok(device_infos)
    }
}

impl CpalAudioCapture {
    fn find_best_config(
        device: &Device,
        config: &AudioCaptureConfig,
    ) -> Result<(StreamConfig, SampleFormat), AudioCaptureError> {
        let supported_configs = device
            .supported_input_configs()
            .map_err(|e| AudioCaptureError::Device(format!("Failed to get configs: {}", e)))?;

        let mut best_config: Option<SupportedStreamConfig> = None;
        let mut best_score = 0i32;
        let mut config_debug = Vec::new();

        for supported_config in supported_configs {
            let mut score = 0i32;

            // Log supported config for debugging
            let min_rate = supported_config.min_sample_rate().0;
            let max_rate = supported_config.max_sample_rate().0;
            let format = supported_config.sample_format();
            let channels = supported_config.channels();
            config_debug.push(format!(
                "  Format: {:?}, Channels: {}, Sample Rate: {}-{} Hz",
                format, channels, min_rate, max_rate
            ));

            // Check if our desired sample rate is in the supported range
            if min_rate <= config.sample_rate && max_rate >= config.sample_rate {
                score += 1000; // Strong preference for exact match
            } else {
                // If exact match not possible, find the best fallback
                // Prefer 48kHz as it's a common rate that can be downsampled to 16kHz
                if min_rate <= 48000 && max_rate >= 48000 {
                    score += 500; // Good fallback
                } else if min_rate <= 44100 && max_rate >= 44100 {
                    score += 400; // Acceptable fallback
                } else {
                    continue; // Skip configs that aren't useful
                }
            }

            // Prefer i16 format
            match format {
                SampleFormat::I16 => score += 50,
                SampleFormat::F32 => score += 25,
                _ => score += 0,
            }

            // Prefer matching channel count, but don't be too strict
            if channels >= config.channels as u16 {
                score += 10;
            }

            if score > best_score {
                best_score = score;

                // Choose the best sample rate to use
                let target_rate =
                    if min_rate <= config.sample_rate && max_rate >= config.sample_rate {
                        config.sample_rate // Use exact rate if supported
                    } else if min_rate <= 48000 && max_rate >= 48000 {
                        48000 // Use 48kHz as fallback
                    } else if min_rate <= 44100 && max_rate >= 44100 {
                        44100 // Use 44.1kHz as fallback
                    } else {
                        max_rate // Use whatever is available
                    };

                best_config =
                    Some(supported_config.with_sample_rate(cpal::SampleRate(target_rate)));
            }
        }

        log::debug!("CPAL: Available audio configurations:");
        for debug_line in &config_debug {
            log::debug!("{}", debug_line);
        }

        let best = best_config.ok_or_else(|| {
            AudioCaptureError::Config(format!(
                "No compatible audio format found for sample rate {} Hz. Available configs:\n{}",
                config.sample_rate,
                config_debug.join("\n")
            ))
        })?;

        // The config is already set to our chosen sample rate
        let sample_format = best.sample_format();
        let stream_config = best.config();

        log::info!(
            "CPAL: Selected config - Format: {:?}, Channels: {}, Sample Rate: {} Hz (requested: {} Hz)",
            sample_format,
            stream_config.channels,
            stream_config.sample_rate.0,
            config.sample_rate
        );

        Ok((stream_config, sample_format))
    }

    fn build_stream<T>(&self) -> Result<Stream, AudioCaptureError>
    where
        T: Sample + Send + 'static + cpal::SizedSample,
    {
        let buffer = Arc::clone(&self.buffer);
        let resample_buffer = Arc::clone(&self.resample_buffer);
        let is_active = Arc::clone(&self.is_active);
        let channels = self.stream_config.channels as usize;
        let target_channel = self.config.target_channel as usize;
        let resample_ratio = self.resample_ratio;
        let target_chunk_size = 1280; // Expected by detection pipeline

        let stream = self
            .device
            .build_input_stream(
                &self.stream_config,
                move |data: &[T], _: &cpal::InputCallbackInfo| {
                    if !*is_active.lock().unwrap() {
                        return;
                    }

                    // Convert samples to f32 and extract target channel
                    let f32_samples: Vec<f32> = if channels == 1 {
                        data.iter()
                            .map(|&sample| {
                                if std::mem::size_of::<T>() == std::mem::size_of::<f32>() {
                                    unsafe { std::mem::transmute_copy(&sample) }
                                } else if std::mem::size_of::<T>() == std::mem::size_of::<i16>() {
                                    let i_val: i16 = unsafe { std::mem::transmute_copy(&sample) };
                                    i_val as f32 / 32768.0
                                } else {
                                    0.0
                                }
                            })
                            .collect()
                    } else {
                        data.chunks(channels)
                            .filter_map(|chunk| {
                                chunk.get(target_channel).map(|&sample| {
                                    if std::mem::size_of::<T>() == std::mem::size_of::<f32>() {
                                        unsafe { std::mem::transmute_copy(&sample) }
                                    } else if std::mem::size_of::<T>() == std::mem::size_of::<i16>()
                                    {
                                        let i_val: i16 =
                                            unsafe { std::mem::transmute_copy(&sample) };
                                        i_val as f32 / 32768.0
                                    } else {
                                        0.0
                                    }
                                })
                            })
                            .collect()
                    };

                    // Add to resample buffer
                    let mut resample_buf = resample_buffer.lock().unwrap();
                    resample_buf.extend_from_slice(&f32_samples);

                    // Process resampling and accumulate until we have target_chunk_size
                    if (resample_ratio - 1.0).abs() > 0.001 {
                        // We need resampling - but be more aggressive about processing smaller chunks
                        // Calculate minimum input needed for a reasonable output chunk
                        let min_input_for_processing = (320 as f32 * resample_ratio) as usize; // Process smaller chunks more frequently

                        while resample_buf.len() >= min_input_for_processing {
                            // Process what we have, but limit to avoid huge chunks
                            let available_input = resample_buf
                                .len()
                                .min((target_chunk_size as f32 * resample_ratio) as usize);
                            let chunk: Vec<f32> = resample_buf.drain(..available_input).collect();

                            // Calculate how many output samples we'll get
                            let output_samples = (chunk.len() as f32 / resample_ratio) as usize;

                            // Resample this chunk
                            let resampled = {
                                let mut output = Vec::with_capacity(output_samples);

                                for i in 0..output_samples {
                                    let src_index = i as f32 * resample_ratio;
                                    let src_index_floor = src_index.floor() as usize;
                                    let src_index_ceil = (src_index_floor + 1).min(chunk.len() - 1);
                                    let frac = src_index - src_index_floor as f32;

                                    let sample = if src_index_floor >= chunk.len() {
                                        0.0
                                    } else if src_index_ceil >= chunk.len() || frac < 0.001 {
                                        chunk[src_index_floor]
                                    } else {
                                        chunk[src_index_floor] * (1.0 - frac)
                                            + chunk[src_index_ceil] * frac
                                    };

                                    output.push((sample.clamp(-1.0, 1.0) * 32767.0) as i16);
                                }
                                output
                            };

                            // Add the resampled chunk to output buffer
                            let mut buffer = buffer.lock().unwrap();
                            for sample in resampled {
                                buffer.push_back(sample);
                            }
                        }
                    } else {
                        // No resampling needed - process smaller chunks for lower latency
                        let min_chunk_size = 320; // Process 20ms chunks for lower latency
                        while resample_buf.len() >= min_chunk_size {
                            let chunk_size = resample_buf.len().min(target_chunk_size);
                            let chunk: Vec<f32> = resample_buf.drain(..chunk_size).collect();
                            let i16_samples: Vec<i16> = chunk
                                .iter()
                                .map(|&s| (s.clamp(-1.0, 1.0) * 32767.0) as i16)
                                .collect();

                            let mut buffer = buffer.lock().unwrap();
                            for sample in i16_samples {
                                buffer.push_back(sample);
                            }
                        }
                    }

                    // Buffer overflow protection
                    let mut buffer = buffer.lock().unwrap();
                    if buffer.len() > 160000 {
                        buffer.drain(..80000);
                    }
                },
                |err| log::error!("CPAL stream error: {}", err),
                None,
            )
            .map_err(|e| AudioCaptureError::Stream(format!("Failed to build stream: {}", e)))?;

        Ok(stream)
    }
}
