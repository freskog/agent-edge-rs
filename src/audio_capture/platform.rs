// Platform selection logic for audio capture backends

// Linux: PulseAudio only
#[cfg(target_os = "linux")]
pub mod imp {
    pub use crate::audio_capture::imp_pulse::PulseAudioCapture as PlatformAudioCapture;
}

// macOS and Windows: CPAL only
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub mod imp {
    pub use crate::audio_capture::imp_cpal::CpalAudioCapture as PlatformAudioCapture;
}

// Fallback for unsupported platforms
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub mod imp {
    // Fallback implementation that errors at runtime
    use super::super::{
        AudioCapture, AudioCaptureConfig, AudioCaptureError, AudioCaptureStats, AudioDeviceInfo,
    };

    pub struct PlatformAudioCapture;

    impl AudioCapture for PlatformAudioCapture {
        fn new(_config: AudioCaptureConfig) -> Result<Self, AudioCaptureError> {
            #[cfg(target_os = "linux")]
            let msg = "No audio backend available. Install 'pulse' or 'cpal' feature for Linux.";
            #[cfg(target_os = "macos")]
            let msg = "No audio backend available. Install 'cpal' feature for macOS.";
            #[cfg(target_os = "windows")]
            let msg = "No audio backend available. Install 'cpal' feature for Windows.";
            #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
            let msg = "Unsupported platform for audio capture.";

            Err(AudioCaptureError::Config(msg.to_string()))
        }

        fn start(&mut self) -> Result<(), AudioCaptureError> {
            unreachable!()
        }
        fn stop(&mut self) -> Result<(), AudioCaptureError> {
            unreachable!()
        }
        fn read_chunk(&mut self) -> Result<Vec<i16>, AudioCaptureError> {
            unreachable!()
        }
        fn is_active(&self) -> bool {
            unreachable!()
        }
        fn available_samples(&self) -> usize {
            unreachable!()
        }
        fn config(&self) -> &AudioCaptureConfig {
            unreachable!()
        }
        async fn record_for_duration(
            &mut self,
            _duration_secs: f32,
        ) -> Result<Vec<i16>, AudioCaptureError> {
            unreachable!()
        }
        fn get_stats(&self) -> AudioCaptureStats {
            unreachable!()
        }
        fn list_devices(&self) -> Result<Vec<AudioDeviceInfo>, AudioCaptureError> {
            unreachable!()
        }
    }
}

// Re-export the selected platform implementation
pub use imp::PlatformAudioCapture;
