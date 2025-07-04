use super::{ToolError, ToolResult};
use crate::tts::ElevenLabsTTS;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

/// Send a message to the user via text-to-speech
/// This tool enables the LLM to provide feedback when other tools execute silently
pub async fn tell_user(
    arguments: Value,
    cancel_token: CancellationToken,
) -> Result<ToolResult, ToolError> {
    // Check for cancellation
    if cancel_token.is_cancelled() {
        return Err(ToolError::Cancelled);
    }

    // Extract the message parameter
    let message = arguments
        .get("message")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            ToolError::InvalidParameters("Missing required 'message' parameter".to_string())
        })?;

    // Validate message is not empty
    if message.trim().is_empty() {
        return Err(ToolError::InvalidParameters(
            "Message cannot be empty".to_string(),
        ));
    }

    // Return the message for TTS output after speaking it if we have a TTS engine
    if let Some(tts) = ElevenLabsTTS::global() {
        if let Err(e) = tts.synthesize(message, cancel_token.clone()).await {
            // If it's a cancellation, propagate that
            if e.to_string().contains("cancelled") {
                return Err(ToolError::Cancelled);
            }
            // Otherwise, log the error but still return the message
            log::error!("TTS synthesis failed: {}", e);
            log::warn!("Continuing without audio output");
        }
    } else {
        log::warn!(
            "tell_user invoked but no global TTS instance registered – skipping audio output"
        );
    }

    Ok(ToolResult::Response(message.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn test_tell_user() {
        let cancel_token = CancellationToken::new();
        let result = tell_user(json!({"message": "Hello, user!"}), cancel_token).await;

        assert!(result.is_ok());

        if let Ok(ToolResult::Response(message)) = result {
            assert_eq!(message, "Hello, user!");
        } else {
            panic!("Expected Response result with message");
        }
    }

    #[tokio::test]
    async fn test_tell_user_missing_message() {
        let cancel_token = CancellationToken::new();
        let result = tell_user(json!({}), cancel_token).await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ToolError::InvalidParameters(_)
        ));
    }

    #[tokio::test]
    async fn test_tell_user_empty_message() {
        let cancel_token = CancellationToken::new();
        let result = tell_user(json!({"message": ""}), cancel_token).await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ToolError::InvalidParameters(_)
        ));
    }

    #[tokio::test]
    async fn test_tell_user_cancellation() {
        let cancel_token = CancellationToken::new();
        cancel_token.cancel(); // Cancel immediately

        let result = tell_user(json!({"message": "This should be cancelled"}), cancel_token).await;

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ToolError::Cancelled));
    }
}
