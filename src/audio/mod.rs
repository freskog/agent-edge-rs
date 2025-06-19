pub mod channel;

// PulseAudio capture (Linux only, requires pulse feature)
#[cfg(all(target_os = "linux", feature = "pulse"))]
pub mod pulse_capture;

pub use channel::ChannelExtractor;

// Conditional re-export for PulseAudio types
#[cfg(all(target_os = "linux", feature = "pulse"))]
pub use pulse_capture::{PulseAudioCapture, PulseAudioCaptureConfig};

// Re-export common audio types
pub type AudioSample = f32;
pub type AudioBuffer = Vec<AudioSample>;
