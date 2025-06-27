use thiserror::Error;

#[derive(Error, Debug)]
pub enum AudioCaptureError {
    #[error("Audio device error: {0}")]
    Device(String),
    #[error("Audio stream error: {0}")]
    Stream(String),
    #[error("Configuration error: {0}")]
    Config(String),
    #[error("No audio data available")]
    NoData,
    #[error("Audio capture not started")]
    NotStarted,
    #[error("Audio capture already started")]
    AlreadyStarted,
}

/// Audio capture configuration compatible with detection pipeline
#[derive(Debug, Clone)]
pub struct AudioCaptureConfig {
    /// Sample rate in Hz (detection pipeline requires 16000)
    pub sample_rate: u32,
    /// Number of input channels (ReSpeaker has 6, most devices have 1-2)
    pub channels: u8,
    /// Target channel to extract (0 = first channel)
    pub target_channel: u8,
    /// Device name (None = default device)
    pub device_name: Option<String>,
    /// Target latency in milliseconds
    pub target_latency_ms: u32,
    /// Application name for audio system
    pub app_name: String,
    /// Stream name for audio system
    pub stream_name: String,
}

impl Default for AudioCaptureConfig {
    fn default() -> Self {
        Self {
            sample_rate: 16000, // Required by detection pipeline
            channels: 1,        // Most common case
            target_channel: 0,  // First channel
            device_name: None,  // Default device
            target_latency_ms: 50,
            app_name: "agent-edge".to_string(),
            stream_name: "audio-capture".to_string(),
        }
    }
}

/// Audio device information
#[derive(Debug, Clone)]
pub struct AudioDeviceInfo {
    pub name: String,
    pub id: String,
    pub is_default: bool,
    pub max_channels: u32,
    pub supported_sample_rates: Vec<u32>,
}

/// Audio capture statistics
#[derive(Debug, Clone)]
pub struct AudioCaptureStats {
    pub total_samples_captured: u64,
    pub current_sample_rate: u32,
    pub current_channels: u32,
    pub buffer_underruns: u32,
    pub buffer_overruns: u32,
}

/// Unified audio capture trait that all platforms implement
pub trait AudioCapture {
    /// Create new audio capture with configuration
    fn new(config: AudioCaptureConfig) -> Result<Self, AudioCaptureError>
    where
        Self: Sized;

    /// Start audio capture
    fn start(&mut self) -> Result<(), AudioCaptureError>;

    /// Stop audio capture
    fn stop(&mut self) -> Result<(), AudioCaptureError>;

    /// Read audio chunk in detection pipeline format: Vec<i16>
    fn read_chunk(&mut self) -> Result<Vec<i16>, AudioCaptureError>;

    /// Check if capture is currently active
    fn is_active(&self) -> bool;

    /// Get available sample count
    fn available_samples(&self) -> usize;

    /// Get the current configuration
    fn config(&self) -> &AudioCaptureConfig;

    /// Record for a specific duration (async version)
    fn record_for_duration(
        &mut self,
        duration_secs: f32,
    ) -> impl std::future::Future<Output = Result<Vec<i16>, AudioCaptureError>>;

    /// Get audio statistics
    fn get_stats(&self) -> AudioCaptureStats;

    /// List available audio devices
    fn list_devices(&self) -> Result<Vec<AudioDeviceInfo>, AudioCaptureError>;
}

// Implementation modules - platform specific
#[cfg(target_os = "linux")]
pub mod imp_pulse;

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub mod imp_cpal;

// Platform selection logic
pub mod platform;

// Re-export the platform-specific implementation
pub use platform::PlatformAudioCapture;
