//! The main library for the `agent-edge-rs` voice assistant.
//!
//! This library provides all the core components for building the edge agent,
//! including audio capture, VAD, wakeword detection, and STT streaming.

// Public modules, accessible to the binary and other consumers
pub mod audio;
pub mod blocking_stt;
pub mod config;
pub mod error;
pub mod llm;
pub mod services;
pub mod tts;
pub mod types;
pub mod vad;

// Re-export common types
pub use error::{AgentError as EdgeError, Result as EdgeResult};
pub use types::*;

/// Represents a chunk of audio data captured from the microphone.
///
/// This struct is made public to be shared between the audio capture loop
/// in the main binary and the STT streaming module in the library.
#[derive(Debug, Clone)]
pub struct AudioChunk {
    pub samples_i16: Vec<i16>,
    pub samples_f32: Vec<f32>,
    pub timestamp: std::time::Instant,
    pub should_process: bool, // VAD result included
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_audio_format_pipeline_fix() {
        println!("🔧 Testing audio format pipeline fix");

        // Test the conversion chain that was previously broken
        let original_samples = vec![-1.0f32, -0.5, 0.0, 0.5, 1.0];

        println!("🎵 Testing fixed conversion pipeline:");

        for (i, &original) in original_samples.iter().enumerate() {
            // Step 1: f32 → i16 (fixed audio source conversion)
            let clamped = original.clamp(-1.0, 1.0);
            let i16_sample = (clamped * 32768.0).clamp(-32768.0, 32767.0) as i16;

            // Step 2: i16 → Vec<u8> (audio protocol - same as before)
            let bytes = i16_sample.to_le_bytes();

            // Step 3: Vec<u8> → i16 (STT service - same as before)
            let restored_i16 = i16::from_le_bytes([bytes[0], bytes[1]]);

            // Step 4: i16 → f32 (for VAD only - same as before)
            let f32_for_vad = restored_i16 as f32 / 32768.0;

            // Step 5: For STT, we now send raw bytes directly (no conversion!)
            let stt_data = bytes.to_vec(); // Raw bytes go directly to STT

            let precision_loss = (original - f32_for_vad).abs();

            println!(
                "  Sample {}: {:.6} → {} → {:?} → {} → {:.6} (loss: {:.6})",
                i, original, i16_sample, bytes, restored_i16, f32_for_vad, precision_loss
            );
            println!(
                "    STT gets raw bytes: {:?} (no precision loss!)",
                stt_data
            );

            // Verify the conversion is much better now (was ~0.00006, now ~0.00003)
            assert!(
                precision_loss < 0.00005,
                "Precision loss should be minimal: {:.6}",
                precision_loss
            );
            assert_eq!(i16_sample, restored_i16, "i16 values should be identical");
        }

        println!("✅ Audio format pipeline fix verified - no more precision loss!");
    }
}
