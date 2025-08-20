use clap::ValueEnum;

/// Supported audio platforms with different characteristics
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum AudioPlatform {
    /// Raspberry Pi with RespeakerPi USB array and I2S DAC
    #[value(name = "raspberry-pi")]
    RaspberryPi,
    /// macOS with built-in audio hardware
    #[value(name = "macos")]
    MacOS,
}

impl AudioPlatform {
    /// Get the platform configuration for audio capture
    pub fn capture_config(self) -> PlatformCaptureConfig {
        match self {
            AudioPlatform::RaspberryPi => PlatformCaptureConfig {
                // RespeakerPi 4-mic array: 6 channels, 16kHz, i16
                preferred_sample_rate: 16000,
                preferred_format: PlatformSampleFormat::I16,
                channel_count: 6,
                target_channel: 0, // Extract channel 0
                description: "RespeakerPi 4-mic USB array",
            },
            AudioPlatform::MacOS => PlatformCaptureConfig {
                // macOS built-in: typically f32, 44.1kHz, mono
                preferred_sample_rate: 44100,
                preferred_format: PlatformSampleFormat::F32,
                channel_count: 1,
                target_channel: 0,
                description: "macOS built-in microphone",
            },
        }
    }

    /// Get the platform configuration for audio playback
    pub fn playback_config(self) -> PlatformPlaybackConfig {
        match self {
            AudioPlatform::RaspberryPi => PlatformPlaybackConfig {
                // I2S DAC: 48kHz, i16, stereo (better scaling from 16kHz)
                sample_rate: 48000,
                format: PlatformSampleFormat::I16,
                channels: 2, // Stereo output
                description: "Raspberry Pi I2S DAC",
            },
            AudioPlatform::MacOS => PlatformPlaybackConfig {
                // macOS speakers: 48kHz, f32, stereo (better scaling from 16kHz)
                sample_rate: 48000,
                format: PlatformSampleFormat::F32,
                channels: 2, // Stereo output
                description: "macOS built-in speakers",
            },
        }
    }

    /// Get the standard TTS output format (same for both platforms)
    pub fn tts_format(self) -> AudioFormatSpec {
        // Standardized TTS output: 48kHz, i16, mono (better scaling from 16kHz)
        AudioFormatSpec {
            sample_rate: 48000,
            format: PlatformSampleFormat::I16,
            channels: 1, // TTS is mono
        }
    }

    /// Get the standard STT/Wakeword input format (same for both platforms)  
    pub fn stt_format(self) -> AudioFormatSpec {
        // STT/Wakeword requirement: 16kHz, i16, mono
        AudioFormatSpec {
            sample_rate: 16000,
            format: PlatformSampleFormat::I16,
            channels: 1,
        }
    }
}

/// Platform-specific sample format (simplified)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformSampleFormat {
    I16, // 16-bit signed integer
    F32, // 32-bit float
}

/// Platform-specific audio capture configuration
#[derive(Debug, Clone)]
pub struct PlatformCaptureConfig {
    /// Preferred sample rate for this platform's hardware
    pub preferred_sample_rate: u32,
    /// Native sample format for this platform
    pub preferred_format: PlatformSampleFormat,
    /// Number of channels the hardware provides
    pub channel_count: u32,
    /// Which channel to extract (0-based)
    pub target_channel: u32,
    /// Human-readable description
    pub description: &'static str,
}

/// Platform-specific audio playback configuration
#[derive(Debug, Clone)]
pub struct PlatformPlaybackConfig {
    /// Target sample rate for playback
    pub sample_rate: u32,
    /// Target sample format for playback
    pub format: PlatformSampleFormat,
    /// Number of output channels
    pub channels: u32,
    /// Human-readable description
    pub description: &'static str,
}

/// Generic audio format specification
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioFormatSpec {
    pub sample_rate: u32,
    pub format: PlatformSampleFormat,
    pub channels: u32,
}

impl std::fmt::Display for AudioPlatform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AudioPlatform::RaspberryPi => write!(f, "Raspberry Pi"),
            AudioPlatform::MacOS => write!(f, "macOS"),
        }
    }
}

impl std::fmt::Display for PlatformSampleFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlatformSampleFormat::I16 => write!(f, "i16"),
            PlatformSampleFormat::F32 => write!(f, "f32"),
        }
    }
}
