use crate::error::AgentError;
use crate::services::TTSService;
use crate::tts::ElevenLabsTTS;
use std::sync::Arc;

/// ElevenLabs TTS service implementation (now blocking)
pub struct ElevenLabsTTSService {
    tts: Arc<ElevenLabsTTS>,
}

impl ElevenLabsTTSService {
    pub fn new(api_key: String, audio_address: String) -> Result<Self, AgentError> {
        let mut tts = ElevenLabsTTS::new(api_key);
        // Store audio address for blocking synthesis
        tts.audio_address = audio_address;

        Ok(Self { tts: Arc::new(tts) })
    }
}

impl TTSService for ElevenLabsTTSService {
    /// Speak the given text (now blocking)
    fn speak(&self, text: String) -> Result<(), AgentError> {
        log::info!("🔊 Speaking text: '{}'", text);

        // Use blocking synthesis
        self.tts
            .synthesize_blocking(&text)
            .map_err(|e| AgentError::TTS(format!("TTS synthesis failed: {}", e)))?;

        log::info!("✅ TTS synthesis completed");
        Ok(())
    }
}
