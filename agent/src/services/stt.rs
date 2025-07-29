use crate::blocking_stt::{types::RawChunk, BlockingSTTService};
use crate::error::AgentError;
use audio_protocol::client::AudioClient;
use std::collections::VecDeque;
use std::time::Instant;

/// Simple STT Service - maintains continuous audio buffer, single connection
pub struct STTService {
    blocking_stt: BlockingSTTService,
    audio_client: Option<AudioClient>,
    audio_buffer: VecDeque<audio_protocol::AudioChunk>,
    max_buffer_chunks: usize,
}

impl STTService {
    /// Create a new STT service
    pub fn new(blocking_stt: BlockingSTTService) -> Result<Self, AgentError> {
        log::info!("🎤 Initializing simple STT service");

        Ok(Self {
            blocking_stt,
            audio_client: None,
            audio_buffer: VecDeque::new(),
            max_buffer_chunks: 64, // ~2 seconds at 32ms per chunk
        })
    }

    /// Set the audio client and start continuous buffering
    pub fn set_audio_client(&mut self, mut audio_client: AudioClient) {
        log::info!("🎧 Setting up audio client for STT service");

        // Subscribe to audio immediately
        if let Err(e) = audio_client.subscribe_audio() {
            log::error!("❌ Failed to subscribe to audio: {}", e);
            return;
        }

        self.audio_client = Some(audio_client);
        log::info!("✅ Audio client ready for STT service");
    }

    /// Continuously update the rolling buffer (call this periodically)
    fn update_buffer(&mut self) -> Result<(), AgentError> {
        let client = self
            .audio_client
            .as_mut()
            .ok_or_else(|| AgentError::STT("No audio client available".to_string()))?;

        // Read available chunks (non-blocking)
        let mut chunks_read = 0;
        loop {
            match client.read_audio_chunk() {
                Ok(Some(chunk)) => {
                    // Add to rolling buffer
                    self.audio_buffer.push_back(chunk);
                    chunks_read += 1;

                    // Keep buffer size limited (rolling window)
                    while self.audio_buffer.len() > self.max_buffer_chunks {
                        self.audio_buffer.pop_front();
                    }
                }
                Ok(None) => {
                    // No more chunks available right now
                    break;
                }
                Err(e) => {
                    log::warn!("⚠️ Error reading audio chunk: {}", e);
                    break;
                }
            }
        }

        if chunks_read > 0 {
            log::trace!(
                "📥 Buffer updated: {} new chunks, {} total chunks",
                chunks_read,
                self.audio_buffer.len()
            );
        }

        Ok(())
    }
}

impl crate::services::STTService for STTService {
    /// Start continuous audio buffering
    fn start_audio_buffering(&mut self) -> Result<(), AgentError> {
        match &self.audio_client {
            Some(_) => {
                // Fill initial buffer
                self.update_buffer()?;
                log::info!("🎤 Audio buffering started (continuous rolling buffer)");
                Ok(())
            }
            None => Err(AgentError::STT("No audio client available".to_string())),
        }
    }

    /// Transcribe speech from wakeword detection
    fn transcribe_from_wakeword(&mut self) -> Result<String, AgentError> {
        log::info!("🎯 Starting STT transcription from wakeword");

        // Update buffer one more time to get latest audio
        self.update_buffer()?;

        // Get context chunks from our rolling buffer
        let context_chunks: Vec<RawChunk> = self
            .audio_buffer
            .iter()
            .map(|chunk| {
                RawChunk::new(
                    chunk.data.clone(),
                    Instant::now(),
                    crate::blocking_stt::types::SpeechEvent::Speech,
                )
            })
            .collect();

        log::info!(
            "📝 Using {} context chunks from rolling buffer (~{:.1}s of audio)",
            context_chunks.len(),
            context_chunks.len() as f32 * 0.032 // ~32ms per chunk
        );

        // Take ownership of the audio client for STT processing
        let audio_client = self
            .audio_client
            .take()
            .ok_or_else(|| AgentError::STT("No audio client available".to_string()))?;

        // Use the SAME audio client for STT (no double connections!)
        let result = self
            .blocking_stt
            .transcribe_from_wakeword(audio_client, context_chunks);

        // Note: We consumed the audio client, so we'll need to reconnect later
        // This is intentional to avoid resource leaks

        match result {
            Ok(transcript) => {
                log::info!("✅ STT transcription successful: '{}'", transcript);
                Ok(transcript)
            }
            Err(e) => {
                log::error!("❌ STT transcription failed: {}", e);
                Err(AgentError::STT(format!("Transcription failed: {}", e)))
            }
        }
    }
}
