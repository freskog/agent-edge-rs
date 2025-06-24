pub mod channel;

// PulseAudio capture (Linux only)
pub mod pulse_capture;

pub use channel::ChannelExtractor;

// Re-export PulseAudio types
pub use pulse_capture::{PulseAudioCapture, PulseAudioCaptureConfig};

// Re-export common audio types
pub type AudioSample = f32;
pub type AudioBuffer = Vec<AudioSample>;
