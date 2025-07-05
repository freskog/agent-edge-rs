//! Test utilities for wakeword detection tests
//!
//! This module provides utilities for loading and processing test audio files.

use crate::error::{OpenWakeWordError, Result};
use std::path::Path;

/// Load a WAV audio file and return it as Vec<i16> samples
///
/// This function loads WAV files in the format expected by our models:
/// - 16kHz sample rate
/// - Mono (1 channel)  
/// - 16-bit signed integer samples (pcm_s16le)
///
/// # Arguments
/// * `file_path` - Path to the WAV file
///
/// # Returns
/// * `Result<Vec<i16>>` - Audio samples as 16-bit signed integers
pub fn load_test_audio<P: AsRef<Path>>(file_path: P) -> Result<Vec<i16>> {
    let path = file_path.as_ref();

    // Open WAV file
    let mut reader = hound::WavReader::open(path).map_err(|e| {
        OpenWakeWordError::IoError(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("Failed to open WAV file {}: {}", path.display(), e),
        ))
    })?;

    let spec = reader.spec();

    // Validate audio format
    if spec.channels != 1 {
        return Err(OpenWakeWordError::InvalidInput(format!(
            "Audio file {} must be mono (1 channel), got {} channels",
            path.display(),
            spec.channels
        )));
    }

    if spec.sample_rate != 16000 {
        return Err(OpenWakeWordError::InvalidInput(format!(
            "Audio file {} must be 16kHz, got {}Hz",
            path.display(),
            spec.sample_rate
        )));
    }

    if spec.bits_per_sample != 16 {
        return Err(OpenWakeWordError::InvalidInput(format!(
            "Audio file {} must be 16-bit, got {} bits per sample",
            path.display(),
            spec.bits_per_sample
        )));
    }

    // Read samples
    let samples: Vec<i16> = reader
        .samples::<i16>()
        .map(|sample| {
            sample.map_err(|e| {
                OpenWakeWordError::IoError(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("Failed to read audio sample from {}: {}", path.display(), e),
                ))
            })
        })
        .collect::<Result<Vec<_>>>()?;

    println!(
        "Loaded {} samples from {} ({:.2}s at 16kHz)",
        samples.len(),
        path.display(),
        samples.len() as f32 / 16000.0
    );

    Ok(samples)
}

/// Generate test audio samples for testing
///
/// # Arguments
/// * `duration_ms` - Duration in milliseconds
/// * `sample_rate` - Sample rate (default: 16000)
/// * `amplitude` - Amplitude of the generated signal (default: 1000)
///
/// # Returns
/// * `Vec<i16>` - Generated audio samples
pub fn generate_test_audio(duration_ms: u32, sample_rate: u32, amplitude: i16) -> Vec<i16> {
    let total_samples = (sample_rate as u64 * duration_ms as u64 / 1000) as usize;

    (0..total_samples)
        .map(|i| {
            // Generate a simple sine wave for testing
            let t = i as f32 / sample_rate as f32;
            let frequency = 440.0; // A4 note
            let sample =
                (amplitude as f32 * (2.0 * std::f32::consts::PI * frequency * t).sin()) as i16;
            sample
        })
        .collect()
}

/// Generate silence (zeros) for testing
///
/// # Arguments
/// * `duration_ms` - Duration in milliseconds
/// * `sample_rate` - Sample rate (default: 16000)
///
/// # Returns
/// * `Vec<i16>` - Silent audio samples (all zeros)
pub fn generate_silence(duration_ms: u32, sample_rate: u32) -> Vec<i16> {
    let total_samples = (sample_rate as u64 * duration_ms as u64 / 1000) as usize;
    vec![0i16; total_samples]
}

/// Generate random noise for testing
///
/// # Arguments  
/// * `duration_ms` - Duration in milliseconds
/// * `sample_rate` - Sample rate (default: 16000)
/// * `amplitude` - Maximum amplitude of noise
///
/// # Returns
/// * `Vec<i16>` - Random noise samples
pub fn generate_noise(duration_ms: u32, sample_rate: u32, amplitude: i16) -> Vec<i16> {
    let total_samples = (sample_rate as u64 * duration_ms as u64 / 1000) as usize;

    (0..total_samples)
        .map(|_| {
            let noise = rand::random::<f32>() * 2.0 - 1.0; // -1.0 to 1.0
            (noise * amplitude as f32) as i16
        })
        .collect()
}

/// Validate that audio chunk size matches expected format
///
/// # Arguments
/// * `chunk` - Audio chunk to validate
/// * `expected_size` - Expected chunk size (default: 1280 for 80ms at 16kHz)
///
/// # Returns
/// * `Result<()>` - Success if chunk is valid size
pub fn validate_chunk_size(chunk: &[i16], expected_size: usize) -> Result<()> {
    if chunk.len() != expected_size {
        return Err(OpenWakeWordError::InvalidInput(format!(
            "Invalid chunk size: expected {}, got {}",
            expected_size,
            chunk.len()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_test_audio() {
        let audio = generate_test_audio(1000, 16000, 1000); // 1 second
        assert_eq!(audio.len(), 16000); // 16000 samples at 16kHz = 1 second

        // Check that we have some non-zero samples (sine wave)
        let non_zero_count = audio.iter().filter(|&&x| x != 0).count();
        assert!(
            non_zero_count > 15000,
            "Should have mostly non-zero samples for sine wave"
        );
    }

    #[test]
    fn test_generate_silence() {
        let silence = generate_silence(500, 16000); // 0.5 seconds
        assert_eq!(silence.len(), 8000); // 8000 samples at 16kHz = 0.5 second

        // All samples should be zero
        assert!(
            silence.iter().all(|&x| x == 0),
            "Silence should be all zeros"
        );
    }

    #[test]
    fn test_generate_noise() {
        let noise = generate_noise(100, 16000, 1000); // 0.1 seconds
        assert_eq!(noise.len(), 1600); // 1600 samples at 16kHz = 0.1 second

        // Check that we have varied samples (not all the same)
        let unique_samples: std::collections::HashSet<i16> = noise.iter().cloned().collect();
        assert!(
            unique_samples.len() > 100,
            "Noise should have varied samples"
        );
    }

    #[test]
    fn test_validate_chunk_size() {
        let chunk = vec![0i16; 1280];
        assert!(validate_chunk_size(&chunk, 1280).is_ok());

        let wrong_chunk = vec![0i16; 1000];
        assert!(validate_chunk_size(&wrong_chunk, 1280).is_err());
    }
}
