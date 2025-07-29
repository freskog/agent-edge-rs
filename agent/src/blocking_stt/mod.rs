pub mod capture;
pub mod types;
pub mod ws;

use self::types::{RawChunk, STTError, SpeechEvent};
use self::ws::WebSocketSender;
use crate::vad::{VadConfig, VadProcessor};
use audio_protocol::client::{AudioChunk, AudioClient};
use std::time::{Duration, Instant};

/// Trait for audio sources that can provide audio chunks with timeout
pub trait AudioSource {
    type Error: std::fmt::Display + std::fmt::Debug + Send + Sync + 'static;

    fn read_audio_chunk_timeout(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<AudioChunk>, Self::Error>;
}

/// Implement AudioSource for the real AudioClient
impl AudioSource for AudioClient {
    type Error = audio_protocol::protocol::ProtocolError;

    fn read_audio_chunk_timeout(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<AudioChunk>, Self::Error> {
        self.read_audio_chunk_timeout(timeout)
    }
}

#[derive(Clone)]
pub struct BlockingSTTService {
    api_key: String,
}

impl BlockingSTTService {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }

    /// Transcribe audio from wakeword detection with context chunks
    pub fn transcribe_from_wakeword<T: AudioSource>(
        &self,
        mut audio_source: T,
        context_chunks: Vec<RawChunk>,
    ) -> Result<String, STTError> {
        log::info!("🎯 Starting STT transcription with simplified architecture");
        log::info!("📝 Using {} context chunks", context_chunks.len());

        // Single overall timeout for the entire operation
        let overall_timeout = Duration::from_secs(60); // 60 seconds emergency timeout
        let transcription_start = Instant::now();

        // Create WebSocket connection
        let mut ws_sender = WebSocketSender::new(self.api_key.clone())?;

        // Send context chunks first if any
        if !context_chunks.is_empty() {
            log::info!("📤 Sending {} context chunks", context_chunks.len());
            let mut context_data = Vec::new();
            for chunk in context_chunks {
                context_data.extend_from_slice(&chunk.data);
            }
            if !context_data.is_empty() {
                ws_sender.send_audio_data(context_data)?;
            }
        }

        // Initialize VAD processor
        let vad_config = VadConfig::default();
        let mut vad_processor =
            VadProcessor::new(vad_config).map_err(|e| STTError::VadError(format!("{}", e)))?;

        // Audio processing state
        let mut speech_active = false;
        let mut nospeech_duration = Duration::ZERO;
        let nospeech_timeout = Duration::from_secs(4);
        let chunk_duration = Duration::from_millis(32); // ~32ms per 512 samples at 16kHz
        let audio_read_timeout = Duration::from_secs(3);
        let mut final_transcript = String::new();

        // Main audio processing loop
        'audio_loop: loop {
            // Check overall timeout first
            if transcription_start.elapsed() >= overall_timeout {
                log::error!("⏰ Emergency timeout (60s) - transcription took too long");
                return Err(STTError::EmergencyTimeout);
            }

            // Read audio with timeout
            let audio_chunk = match audio_source.read_audio_chunk_timeout(audio_read_timeout) {
                Ok(Some(chunk)) => chunk,
                Ok(None) => {
                    log::warn!("⏰ Audio read timeout (3s) - no audio from server");
                    return Err(STTError::AudioTimeout);
                }
                Err(e) => {
                    log::error!("❌ Audio read error: {}", e);
                    return Err(STTError::AudioError(format!("{}", e)));
                }
            };

            // Convert audio chunk to f32 samples for VAD
            let f32_samples: Vec<f32> = audio_chunk
                .data
                .chunks_exact(2)
                .map(|bytes| {
                    let sample_i16 = i16::from_le_bytes([bytes[0], bytes[1]]);
                    sample_i16 as f32 / 32768.0
                })
                .collect();

            // Process in 512-sample chunks for VAD
            for chunk_512 in f32_samples.chunks(512) {
                if chunk_512.len() == 512 {
                    let mut samples_array = [0.0f32; 512];
                    samples_array.copy_from_slice(chunk_512);

                    // Apply VAD to determine speech event
                    let speech_event = match vad_processor.process_chunk(&samples_array) {
                        Ok(audio_event) => match audio_event {
                            crate::types::AudioEvent::StartedAudio => {
                                log::debug!("🗣️ VAD: Speech started");
                                SpeechEvent::SpeechStarted
                            }
                            crate::types::AudioEvent::Audio => {
                                log::trace!("🎵 VAD: Ongoing speech");
                                SpeechEvent::Speech
                            }
                            crate::types::AudioEvent::StoppedAudio => {
                                log::info!("🔇 VAD: Speech ended (EOS detected)");
                                SpeechEvent::SpeechStopped
                            }
                        },
                        Err(e) => {
                            log::warn!("⚠️ VAD processing failed: {}, treating as no speech", e);
                            SpeechEvent::NoSpeech
                        }
                    };

                    // Handle speech events
                    match speech_event {
                        SpeechEvent::SpeechStarted => {
                            log::info!("🗣️ Speech started");
                            speech_active = true;
                            nospeech_duration = Duration::ZERO;

                            // Send corresponding portion of audio data
                            let start_idx =
                                chunk_512.as_ptr() as usize - f32_samples.as_ptr() as usize;
                            let start_byte = (start_idx / std::mem::size_of::<f32>()) * 2; // Convert to byte index
                            let end_byte = start_byte + 1024; // 512 samples * 2 bytes per sample
                            let audio_segment = audio_chunk
                                .data
                                .get(start_byte..end_byte.min(audio_chunk.data.len()))
                                .unwrap_or(&audio_chunk.data[start_byte..]);
                            ws_sender.send_audio_data(audio_segment.to_vec())?;
                        }

                        SpeechEvent::Speech => {
                            if speech_active {
                                nospeech_duration = Duration::ZERO;

                                // Send corresponding portion of audio data
                                let start_idx =
                                    chunk_512.as_ptr() as usize - f32_samples.as_ptr() as usize;
                                let start_byte = (start_idx / std::mem::size_of::<f32>()) * 2;
                                let end_byte = start_byte + 1024;
                                let audio_segment = audio_chunk
                                    .data
                                    .get(start_byte..end_byte.min(audio_chunk.data.len()))
                                    .unwrap_or(&audio_chunk.data[start_byte..]);
                                ws_sender.send_audio_data(audio_segment.to_vec())?;
                            }
                        }

                        SpeechEvent::SpeechStopped => {
                            log::info!("🔇 EOS detected - sending final audio and ending stream");
                            if speech_active {
                                // Send final audio segment
                                let start_idx =
                                    chunk_512.as_ptr() as usize - f32_samples.as_ptr() as usize;
                                let start_byte = (start_idx / std::mem::size_of::<f32>()) * 2;
                                let end_byte = start_byte + 1024;
                                let audio_segment = audio_chunk
                                    .data
                                    .get(start_byte..end_byte.min(audio_chunk.data.len()))
                                    .unwrap_or(&audio_chunk.data[start_byte..]);
                                ws_sender.send_audio_data(audio_segment.to_vec())?;
                            }
                            log::info!("📡 Audio stream complete - reading final responses");
                            break 'audio_loop; // Exit audio loop
                        }

                        SpeechEvent::NoSpeech => {
                            nospeech_duration += chunk_duration;
                            if nospeech_duration >= nospeech_timeout {
                                log::warn!("⏰ No speech detected for 4 seconds");
                                return Err(STTError::NoSpeechTimeout);
                            }
                            // Don't send NoSpeech chunks
                        }
                    }
                }
            }

            // Check for WebSocket responses while processing audio
            match ws_sender.read_response()? {
                Some(response) => {
                    if !response.is_empty() {
                        // Skip empty timeout responses
                        log::debug!("📥 WebSocket response: {}", response);

                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&response) {
                            // Check for completion marker (Fireworks sends trace_id: "final")
                            if let Some(trace_id) = json.get("trace_id") {
                                if trace_id.as_str() == Some("final") {
                                    log::info!(
                                        "🏁 Completion marker received during audio processing"
                                    );
                                    // Don't break yet, continue until EOS
                                }
                            }

                            // Update transcript from segments
                            if let Some(segments) = json.get("segments").and_then(|s| s.as_array())
                            {
                                let mut combined = String::new();
                                for segment in segments {
                                    if let Some(text) = segment.get("text").and_then(|t| t.as_str())
                                    {
                                        if !combined.is_empty() {
                                            combined.push(' ');
                                        }
                                        combined.push_str(text.trim());
                                    }
                                }
                                if !combined.is_empty() {
                                    final_transcript = combined;
                                    log::debug!("📝 Updated transcript: '{}'", final_transcript);
                                }
                            }

                            // Fallback: direct text field
                            if let Some(text) = json.get("text").and_then(|t| t.as_str()) {
                                if !text.trim().is_empty() {
                                    final_transcript = text.trim().to_string();
                                    log::debug!("📝 Direct transcript: '{}'", final_transcript);
                                }
                            }
                        }
                    }
                }
                None => {
                    // Connection closed during audio processing
                    log::info!("🔚 WebSocket closed during audio processing");
                    return Ok(final_transcript);
                }
            }
        }

        // After audio stream ends, continue reading WebSocket responses for final transcript
        log::info!("⏳ Reading final WebSocket responses...");
        for _attempt in 0..50 {
            // Try for ~5 seconds (50 * 100ms)
            // Check overall timeout
            if transcription_start.elapsed() >= overall_timeout {
                log::error!("⏰ Emergency timeout (60s) while waiting for final transcript");
                return Err(STTError::EmergencyTimeout);
            }

            match ws_sender.read_response()? {
                Some(response) => {
                    if !response.is_empty() {
                        log::debug!("📥 Final response: {}", response);

                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&response) {
                            // Check for completion marker
                            if let Some(trace_id) = json.get("trace_id") {
                                if trace_id.as_str() == Some("final") {
                                    log::info!("🏁 Final completion marker received");
                                    break;
                                }
                            }

                            // Update final transcript
                            if let Some(segments) = json.get("segments").and_then(|s| s.as_array())
                            {
                                let mut combined = String::new();
                                for segment in segments {
                                    if let Some(text) = segment.get("text").and_then(|t| t.as_str())
                                    {
                                        if !combined.is_empty() {
                                            combined.push(' ');
                                        }
                                        combined.push_str(text.trim());
                                    }
                                }
                                if !combined.is_empty() {
                                    final_transcript = combined;
                                }
                            }

                            if let Some(text) = json.get("text").and_then(|t| t.as_str()) {
                                if !text.trim().is_empty() {
                                    final_transcript = text.trim().to_string();
                                }
                            }
                        }
                    }
                }
                None => {
                    // Connection closed
                    log::info!("🔚 WebSocket closed - returning final transcript");
                    break;
                }
            }

            std::thread::sleep(Duration::from_millis(100));
        }

        ws_sender.close()?;

        log::info!("✅ Received final transcript: '{}'", final_transcript);
        log::info!(
            "⏱️ Total transcription time: {:?}",
            transcription_start.elapsed()
        );
        Ok(final_transcript)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_blocking_stt_creation() {
        let service = BlockingSTTService::new("test_key".to_string());
        assert_eq!(service.api_key, "test_key");
    }
}
