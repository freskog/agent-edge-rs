use log::{debug, info};
use voice_activity_detector::VoiceActivityDetector;

/// Configuration for Voice Activity Detection
#[derive(Debug, Clone)]
pub struct VadConfig {
    /// Minimum duration of speech to consider it started (ms)
    pub speech_start_threshold_ms: u64,
    /// Minimum duration of silence to consider speech ended (ms)
    pub speech_end_threshold_ms: u64,
    /// Sample rate (should match audio chunks)
    pub sample_rate: u32,
    /// Chunk size for VAD processing
    pub chunk_size: usize,
}

impl Default for VadConfig {
    fn default() -> Self {
        Self {
            speech_start_threshold_ms: 200, // 200ms of speech to start
            speech_end_threshold_ms: 800,   // 800ms of silence to end
            sample_rate: 16000,             // 16kHz sample rate
            chunk_size: 512,                // 32ms chunks at 16kHz (required by Silero VAD)
        }
    }
}

/// Audio event types for wakeword service
#[derive(Debug, Clone, PartialEq)]
pub enum AudioEvent {
    StartedAudio, // User started speaking
    Audio,        // User is speaking
    StoppedAudio, // User stopped speaking (End of Speech)
}

/// State for tracking speech/silence transitions
#[derive(Debug, Clone, PartialEq)]
enum VadState {
    Silence,
    Speech,
}

/// Voice Activity Detector with state management for speech/silence transitions
pub struct VadProcessor {
    detector: VoiceActivityDetector,
    config: VadConfig,
    current_state: VadState,
    state_duration_ms: u64,
    chunk_duration_ms: u64, // Duration of each audio chunk
    speech_threshold: f32,  // Threshold for speech detection
}

impl VadProcessor {
    /// Create a new VAD processor with the given configuration
    pub fn new(config: VadConfig) -> Result<Self, VadError> {
        // Create the VAD detector using the builder pattern
        let detector = VoiceActivityDetector::builder()
            .chunk_size(config.chunk_size)
            .sample_rate(config.sample_rate as i64)
            .build()
            .map_err(|e| VadError::InitializationError(e.to_string()))?;

        // Calculate chunk duration based on chunk size and sample rate
        let chunk_duration_ms = (config.chunk_size as u64 * 1000) / config.sample_rate as u64;

        info!(
            "🎤 VAD initialized: chunk_size={}, sample_rate={}Hz, chunk_duration={}ms",
            config.chunk_size, config.sample_rate, chunk_duration_ms
        );

        Ok(Self {
            detector,
            config,
            current_state: VadState::Silence,
            state_duration_ms: 0,
            chunk_duration_ms,
            speech_threshold: 0.5, // Default threshold for speech detection
        })
    }

    /// Process an audio chunk and return the appropriate AudioEvent
    ///
    /// # Arguments
    /// * `samples` - Audio samples as f32 in range [-1.0, 1.0] (512 samples for 16kHz)
    ///
    /// # Returns
    /// * `AudioEvent` indicating the state transition
    pub fn process_chunk(&mut self, samples: &[f32; 512]) -> Result<AudioEvent, VadError> {
        // Run VAD detection - the predict method expects an iterator of samples
        let speech_probability = self.detector.predict(samples.iter().copied());

        // Determine if speech is present based on threshold
        let has_speech = speech_probability >= self.speech_threshold;

        debug!(
            "🎤 VAD: speech_prob={:.3}, has_speech={}, current_state={:?}, duration={}ms",
            speech_probability, has_speech, self.current_state, self.state_duration_ms
        );

        // Update state duration
        self.state_duration_ms += self.chunk_duration_ms;

        // Determine the appropriate AudioEvent based on state transitions
        let audio_event = match (self.current_state.clone(), has_speech) {
            // Currently in silence
            (VadState::Silence, true) => {
                // Speech detected in silence
                if self.state_duration_ms >= self.config.speech_start_threshold_ms {
                    // Sufficient duration to confirm speech started
                    self.transition_to_speech();
                    AudioEvent::StartedAudio
                } else {
                    // Not enough duration yet, stay in silence but increment counter
                    AudioEvent::Audio // Treat as ongoing until confirmed
                }
            }
            (VadState::Silence, false) => {
                // Still in silence
                AudioEvent::Audio // During transcription, silence is still "audio"
            }
            // Currently in speech
            (VadState::Speech, false) => {
                // Silence detected in speech
                if self.state_duration_ms >= self.config.speech_end_threshold_ms {
                    // Sufficient silence duration to confirm speech ended
                    self.transition_to_silence();
                    AudioEvent::StoppedAudio
                } else {
                    // Not enough silence yet, continue as speech
                    AudioEvent::Audio
                }
            }
            (VadState::Speech, true) => {
                // Continuing speech
                self.reset_state_duration(); // Reset silence counter
                AudioEvent::Audio
            }
        };

        debug!("🎤 VAD result: {:?}", audio_event);
        Ok(audio_event)
    }

    /// Transition to speech state
    fn transition_to_speech(&mut self) {
        info!("🗣️ VAD: Speech started");
        self.current_state = VadState::Speech;
        self.state_duration_ms = 0;
    }

    /// Transition to silence state
    fn transition_to_silence(&mut self) {
        info!(
            "🔇 VAD: Speech ended ({}ms of silence)",
            self.state_duration_ms
        );
        self.current_state = VadState::Silence;
        self.state_duration_ms = 0;
    }

    /// Reset state duration (used when continuing in current state)
    fn reset_state_duration(&mut self) {
        self.state_duration_ms = 0;
    }

    /// Get current VAD state for debugging
    pub fn current_state(&self) -> &str {
        match self.current_state {
            VadState::Speech => "speech",
            VadState::Silence => "silence",
        }
    }

    /// Get current state duration for debugging
    pub fn state_duration_ms(&self) -> u64 {
        self.state_duration_ms
    }
}

/// Errors that can occur during VAD processing
#[derive(Debug, thiserror::Error)]
pub enum VadError {
    #[error("VAD initialization failed: {0}")]
    InitializationError(String),
    #[error("VAD processing failed: {0}")]
    ProcessingError(String),
}
