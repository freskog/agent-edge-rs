pub mod melspectrogram;
pub mod wakeword;

// Re-export main types for convenient access
pub use melspectrogram::{MelSpectrogramConfig, MelSpectrogramProcessor};
pub use wakeword::{WakewordConfig, WakewordDetection, WakewordDetector};
