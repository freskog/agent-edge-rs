pub mod capture;
pub mod channel;

#[cfg(all(target_os = "linux", feature = "pulseaudio"))]
pub mod pulse_capture;

pub use capture::{AudioCapture, AudioCaptureConfig};
pub use channel::ChannelExtractor;
#[cfg(all(target_os = "linux", feature = "pulseaudio"))]
pub use pulse_capture::{PulseAudioCapture, PulseAudioCaptureConfig};

// Re-export common audio types
pub type AudioSample = f32;
pub type AudioBuffer = Vec<AudioSample>;
