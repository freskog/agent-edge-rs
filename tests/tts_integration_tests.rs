use agent_edge_rs::{
    audio_sink::{AudioSink, RodioConfig, RodioSink, TestSink},
    config::ApiConfig,
    tts::{ElevenLabsTTS, TTSConfig, TTSError},
};
use std::{sync::Arc, time::Duration};
use test_log::test;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;

/// Helper function to get API key or skip test
fn get_api_key_or_skip() -> String {
    match ApiConfig::load() {
        Ok(config) => config.elevenlabs_key().to_string(),
        Err(_) => {
            println!("⚠️  Skipping ElevenLabs tests - API key not found");
            panic!("Test skipped - no API key");
        }
    }
}

#[tokio::test]
async fn test_tts_basic_synthesis() {
    let api_key = get_api_key_or_skip();
    let config = TTSConfig::default();
    let sink = Arc::new(TestSink::new());
    let tts = ElevenLabsTTS::new(api_key, config, Arc::clone(&sink));

    let cancel = CancellationToken::new();
    let result = tts.synthesize("Hello, this is a test.", cancel).await;

    assert!(result.is_ok(), "Basic synthesis should succeed");

    let chunks = sink.get_chunks().await;
    assert!(!chunks.is_empty(), "Should return audio data");

    println!("✅ Basic synthesis test passed");
}

#[tokio::test]
async fn test_tts_with_custom_voice() {
    let api_key = get_api_key_or_skip();
    let mut config = TTSConfig::default();
    config.voice_id = "29vD33N1CtxCmqQRPOHJ".to_string(); // Drew voice
    let sink = Arc::new(TestSink::new());
    let tts = ElevenLabsTTS::new(api_key, config, Arc::clone(&sink));

    let cancel = CancellationToken::new();
    let result = tts.synthesize("Testing custom voice", cancel).await;

    assert!(result.is_ok(), "Custom voice synthesis should succeed");

    let chunks = sink.get_chunks().await;
    assert!(!chunks.is_empty(), "Should return audio data");

    println!("✅ Custom voice test passed");
}

#[tokio::test]
async fn test_tts_error_handling() {
    let api_key = get_api_key_or_skip();
    let config = TTSConfig::default();
    let sink = Arc::new(TestSink::new());
    let tts = ElevenLabsTTS::new(api_key, config, Arc::clone(&sink));

    // Test empty text
    let cancel = CancellationToken::new();
    let result = tts.synthesize("", cancel).await;
    assert!(
        matches!(result, Err(TTSError::ApiError { .. })),
        "Expected API error for empty text"
    );

    // Test invalid API key
    let tts = ElevenLabsTTS::new("invalid_key".to_string(), config, Arc::clone(&sink));
    let cancel = CancellationToken::new();
    let result = tts.synthesize("Test text", cancel).await;
    assert!(
        matches!(result, Err(TTSError::ApiError { .. })),
        "Expected API error for invalid key"
    );
}

#[tokio::test]
async fn test_tts_voice_settings() {
    let api_key = get_api_key_or_skip();
    let mut config = TTSConfig::default();

    // Test with different voice settings
    config.stability = 0.8;
    config.similarity_boost = 0.9;
    config.style = 0.5;

    let sink = Arc::new(TestSink::new());
    let tts = ElevenLabsTTS::new(api_key, config, Arc::clone(&sink));

    let cancel = CancellationToken::new();
    let result = tts.synthesize("Testing voice settings.", cancel).await;
    assert!(
        result.is_ok(),
        "Synthesis failed with custom voice settings"
    );

    let chunks = sink.get_chunks().await;
    assert!(!chunks.is_empty(), "No audio chunks received");
}

#[tokio::test]
async fn test_get_voices() {
    let api_key = get_api_key_or_skip();
    let config = TTSConfig::default();
    let sink = Arc::new(TestSink::new());
    let tts = ElevenLabsTTS::new(api_key, config, sink);

    let result = tts.get_voices().await;
    assert!(result.is_ok(), "Should be able to get voices");

    let voices = result.unwrap();
    assert!(!voices.is_empty(), "Should return at least one voice");

    // Print first few voices for debugging
    for (i, voice) in voices.iter().take(3).enumerate() {
        println!("Voice {}: {} ({})", i, voice.name, voice.voice_id);
    }
}

// RodioSink tests are in a separate module and only run when explicitly requested
#[cfg(test)]
mod rodio_tests {
    use super::*;

    // Helper to create RodioSink with reasonable defaults for testing
    fn create_rodio_sink() -> Arc<RodioSink> {
        let config = RodioConfig {
            buffer_size_ms: 30000,   // 30 second buffer
            low_buffer_warning: 20,  // 20% low warning
            high_buffer_warning: 80, // 80% high warning
        };
        Arc::new(RodioSink::new(config).expect("Failed to create RodioSink"))
    }

    // This test is ignored by default as it produces actual audio output
    #[tokio::test]
    #[ignore]
    async fn test_tts_with_rodio_sink() {
        let api_key = get_api_key_or_skip();
        let config = TTSConfig::default();
        let sink = create_rodio_sink();
        let tts = ElevenLabsTTS::new(api_key, config, sink);

        let cancel = CancellationToken::new();
        let result = tts
            .synthesize(
                "This is a test of the text to speech system with real audio output.",
                cancel,
            )
            .await;
        assert!(result.is_ok(), "Synthesis failed with RodioSink");

        // Give some time for the audio to play
        sleep(Duration::from_secs(3)).await;
    }

    #[tokio::test]
    #[ignore]
    async fn test_tts_cancellation_with_rodio_sink() {
        let api_key = get_api_key_or_skip();
        let config = TTSConfig::default();
        let sink = create_rodio_sink();
        let tts = ElevenLabsTTS::new(api_key, config, sink);

        let cancel = CancellationToken::new();
        let synthesis = tokio::spawn({
            let cancel = cancel.clone();
            async move {
                tts.synthesize(
                    "This is a long text that will be cancelled mid-way through synthesis. \
                    The quick brown fox jumps over the lazy dog. \
                    We'll keep adding more text to ensure we have time to cancel. \
                    Lorem ipsum dolor sit amet, consectetur adipiscing elit.",
                    cancel,
                )
                .await
            }
        });

        // Wait for audio to start playing
        sleep(Duration::from_secs(1)).await;
        cancel.cancel();

        let result = synthesis.await.unwrap();
        assert!(
            matches!(result, Err(TTSError::Cancelled)),
            "Expected cancellation error"
        );

        // Give some time for cleanup
        sleep(Duration::from_millis(500)).await;
    }
}
