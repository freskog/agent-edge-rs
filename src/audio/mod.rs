pub mod channel;
pub mod pulse_capture;

pub use channel::ChannelExtractor;
pub use pulse_capture::{PulseAudioCapture, PulseAudioCaptureConfig};

// Re-export common audio types
pub type AudioSample = f32;
pub type AudioBuffer = Vec<AudioSample>;
