use super::types::{RawChunk, STTStats, SpeechEvent};
use crate::vad::{VadConfig, VadProcessor};
use audio_protocol::client::AudioClient;
use crossbeam_channel::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::time::Instant;

pub struct AudioCapture {
    audio_client: AudioClient,
    vad_processor: VadProcessor,
    chunk_sender: Sender<RawChunk>,
    stats: Arc<Mutex<STTStats>>,
}

impl AudioCapture {
    pub fn new(
        audio_client: AudioClient,
        chunk_sender: Sender<RawChunk>,
        stats: Arc<Mutex<STTStats>>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let vad_config = VadConfig::default(); // 512 samples, 16kHz
        let vad_processor = VadProcessor::new(vad_config)?;

        Ok(Self {
            audio_client,
            vad_processor,
            chunk_sender,
            stats,
        })
    }

    /// Main blocking loop - reads audio chunks, applies VAD, sends to channel
    pub fn run_capture_loop(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        log::info!("🎤 Starting audio capture loop for STT (blocking)");

        // Mark transcription start
        if let Ok(mut stats) = self.stats.lock() {
            stats.transcription_start = Some(Instant::now());
        }

        // Set a maximum capture time to prevent infinite loops
        let max_capture_time = Duration::from_secs(15); // 15 seconds max (reasonable for user speech)
        let capture_start = Instant::now();
        let mut last_audio_time = Instant::now();
        let audio_timeout = Duration::from_secs(3); // 3 seconds of silence = timeout (much more practical)
        let mut last_speech_time: Option<Instant> = None; // Track when we last detected speech

        loop {
            // Check overall timeout
            if capture_start.elapsed() > max_capture_time {
                log::warn!("⏰ Audio capture timeout after {:?}", max_capture_time);
                return Ok(());
            }

            // Read raw audio chunk from the client (blocking)
            match self.audio_client.read_audio_chunk() {
                Ok(Some(protocol_chunk)) => {
                    let timestamp = Instant::now();
                    last_audio_time = timestamp; // Reset audio timeout

                    // Update stats
                    if let Ok(mut stats) = self.stats.lock() {
                        stats.chunks_captured += 1;
                    }

                    // Convert raw bytes to f32 for VAD (i16 -> f32)
                    let f32_samples: Vec<f32> = protocol_chunk
                        .data
                        .chunks_exact(2)
                        .map(|bytes| {
                            let sample_i16 = i16::from_le_bytes([bytes[0], bytes[1]]);
                            sample_i16 as f32 / 32768.0
                        })
                        .collect();

                    // Process in 512-sample chunks for VAD (as required by Silero VAD)
                    for chunk_512 in f32_samples.chunks(512) {
                        if chunk_512.len() == 512 {
                            let mut samples_array = [0.0f32; 512];
                            samples_array.copy_from_slice(chunk_512);

                            // Apply VAD to determine speech event
                            let mut speech_event =
                                match self.vad_processor.process_chunk(&samples_array) {
                                    Ok(audio_event) => match audio_event {
                                        crate::types::AudioEvent::StartedAudio => {
                                            log::debug!("🗣️ VAD: Speech started");
                                            last_speech_time = Some(Instant::now());
                                            SpeechEvent::SpeechStarted
                                        }
                                        crate::types::AudioEvent::Audio => {
                                            log::trace!("🎵 VAD: Ongoing speech");
                                            last_speech_time = Some(Instant::now());
                                            SpeechEvent::Speech
                                        }
                                        crate::types::AudioEvent::StoppedAudio => {
                                            log::info!("🔇 VAD: Speech ended (EOS detected)");
                                            SpeechEvent::SpeechStopped
                                        }
                                    },
                                    Err(e) => {
                                        log::warn!(
                                            "⚠️ VAD processing failed: {}, treating as no speech",
                                            e
                                        );
                                        SpeechEvent::NoSpeech
                                    }
                                };

                            // Additional check: if we had speech but haven't detected any for 2 seconds, force EOS
                            if let Some(last_speech) = last_speech_time {
                                if last_speech.elapsed() > Duration::from_secs(2)
                                    && speech_event != SpeechEvent::SpeechStopped
                                {
                                    log::info!(
                                        "🔇 Forcing EOS: 2 seconds since last speech detected"
                                    );
                                    speech_event = SpeechEvent::SpeechStopped;
                                }
                            }

                            // Create RawChunk with original raw bytes (not the f32 converted ones)
                            let chunk_start_byte = (chunk_512.as_ptr() as usize
                                - f32_samples.as_ptr() as usize)
                                / std::mem::size_of::<f32>()
                                * 2;
                            let chunk_end_byte = chunk_start_byte + (512 * 2); // 512 samples * 2 bytes
                            let chunk_raw_data = protocol_chunk.data
                                [chunk_start_byte..chunk_end_byte.min(protocol_chunk.data.len())]
                                .to_vec();

                            let raw_chunk =
                                RawChunk::new(chunk_raw_data, timestamp, speech_event.clone());

                            // Send to WebSocket thread
                            match self.chunk_sender.try_send(raw_chunk) {
                                Ok(_) => {
                                    if let Ok(mut stats) = self.stats.lock() {
                                        stats.chunks_sent += 1;
                                        stats.bytes_sent += 512 * 2; // 512 samples * 2 bytes
                                    }
                                }
                                Err(crossbeam_channel::TrySendError::Full(_)) => {
                                    log::warn!("📦 Channel full, dropping audio chunk");
                                    if let Ok(mut stats) = self.stats.lock() {
                                        stats.chunks_dropped += 1;
                                    }
                                }
                                Err(crossbeam_channel::TrySendError::Disconnected(_)) => {
                                    log::info!(
                                        "📡 WebSocket thread disconnected, stopping capture"
                                    );
                                    return Ok(());
                                }
                            }

                            // If we detected end of speech, we can break the loop
                            if speech_event == SpeechEvent::SpeechStopped {
                                log::info!("🔇 EOS detected, stopping audio capture");
                                return Ok(());
                            }
                        }
                    }
                }
                Ok(None) => {
                    log::debug!("📭 No audio chunk available, continuing...");

                    // Check for audio timeout (no audio for too long)
                    if last_audio_time.elapsed() > audio_timeout {
                        log::warn!(
                            "⏰ No audio received for {:?}, treating as EOS",
                            audio_timeout
                        );
                        return Ok(());
                    }

                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(e) => {
                    log::error!("❌ Failed to read audio chunk: {}", e);
                    return Err(Box::new(e));
                }
            }
        }
    }
}
