use agent_edge_rs::{
    audio_sink::{AudioSink, CpalConfig, CpalSink},
    tts::{ElevenLabsTTS, TTSConfig, TTSError},
};
use std::sync::Arc;
use tokio::time::Duration;
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn test_tts_basic() -> Result<(), TTSError> {
    let config = TTSConfig {
        voice_id: "21m00Tcm4TlvDq8ikWAM".to_string(),
        model: "eleven_monolingual_v1".to_string(),
        stability: 0.5,
        similarity_boost: 0.75,
        style: 0.0,
        use_speaker_boost: true,
    };

    let sink = match CpalSink::new(CpalConfig::default()) {
        Ok(sink) => Arc::new(sink) as Arc<dyn AudioSink>,
        Err(e) => {
            println!(
                "Audio device not available in test environment - this is expected: {}",
                e
            );
            return Ok(());
        }
    };
    let tts = ElevenLabsTTS::new("test_key".to_string(), config, Arc::clone(&sink));

    let text = "Hello, this is a test.";
    tts.synthesize(text, CancellationToken::new()).await?;

    Ok(())
}

#[tokio::test]
async fn test_tts_empty_text() -> Result<(), TTSError> {
    let config = TTSConfig {
        voice_id: "21m00Tcm4TlvDq8ikWAM".to_string(),
        model: "eleven_monolingual_v1".to_string(),
        stability: 0.5,
        similarity_boost: 0.75,
        style: 0.0,
        use_speaker_boost: true,
    };

    let sink = match CpalSink::new(CpalConfig::default()) {
        Ok(sink) => Arc::new(sink) as Arc<dyn AudioSink>,
        Err(e) => {
            println!(
                "Audio device not available in test environment - this is expected: {}",
                e
            );
            return Ok(());
        }
    };
    let tts = ElevenLabsTTS::new("test_key".to_string(), config, Arc::clone(&sink));

    let result = tts.synthesize("", CancellationToken::new()).await;
    assert!(matches!(result, Err(TTSError::ApiError { .. })));

    Ok(())
}

#[tokio::test]
async fn test_tts_invalid_config() {
    let config = TTSConfig {
        voice_id: "nonexistent_voice".to_string(),
        model: "nonexistent_model".to_string(),
        stability: 0.5,
        similarity_boost: 0.75,
        style: 0.0,
        use_speaker_boost: true,
    };

    let sink = match CpalSink::new(CpalConfig::default()) {
        Ok(sink) => Arc::new(sink) as Arc<dyn AudioSink>,
        Err(e) => {
            println!(
                "Audio device not available in test environment - this is expected: {}",
                e
            );
            return;
        }
    };
    let tts = ElevenLabsTTS::new("test_key".to_string(), config, Arc::clone(&sink));

    let result = tts.synthesize("test", CancellationToken::new()).await;
    assert!(matches!(result, Err(TTSError::ApiError { .. })));
}

#[tokio::test]
async fn test_tts_cancellation() -> Result<(), TTSError> {
    let config = TTSConfig {
        voice_id: "21m00Tcm4TlvDq8ikWAM".to_string(),
        model: "eleven_monolingual_v1".to_string(),
        stability: 0.5,
        similarity_boost: 0.75,
        style: 0.0,
        use_speaker_boost: true,
    };

    let sink = match CpalSink::new(CpalConfig::default()) {
        Ok(sink) => Arc::new(sink) as Arc<dyn AudioSink>,
        Err(e) => {
            println!(
                "Audio device not available in test environment - this is expected: {}",
                e
            );
            return Ok(());
        }
    };
    let tts = ElevenLabsTTS::new("test_key".to_string(), config, Arc::clone(&sink));

    // Start synthesis
    let text = "This is a long text that will be cancelled. ".repeat(10);
    let cancel = CancellationToken::new();
    let synthesis_handle = tokio::spawn({
        let cancel = cancel.clone();
        async move { tts.synthesize(&text, cancel).await }
    });

    // Wait a bit and then cancel
    tokio::time::sleep(Duration::from_millis(100)).await;
    cancel.cancel();

    // Wait for synthesis to complete
    let result = synthesis_handle.await.unwrap();
    assert!(matches!(result, Err(TTSError::Cancelled)));

    Ok(())
}
