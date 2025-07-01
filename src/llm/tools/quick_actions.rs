use super::{ToolError, ToolResult};
use chrono::{DateTime, Duration, Local, Timelike};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

/// Get the current time in a human-readable format
pub async fn get_time(
    arguments: Value,
    cancel_token: CancellationToken,
) -> Result<ToolResult, ToolError> {
    // Check for cancellation
    if cancel_token.is_cancelled() {
        return Err(ToolError::Cancelled);
    }

    // Extract the send_output_directly_to_tts parameter
    let send_directly = arguments
        .get("send_output_directly_to_tts")
        .and_then(|v| v.as_bool())
        .unwrap_or(true); // Default to true for safety

    let now: DateTime<Local> = Local::now();

    // Format time in a natural, conversational way
    let time_str = now.format("%I:%M %p").to_string();
    let time_str = time_str.trim_start_matches('0'); // Remove leading zero from hour

    // Return based on routing preference
    if send_directly {
        Ok(ToolResult::Success(Some(format!("It's {}", time_str))))
    } else {
        // For LLM processing - used for calculations like "what time will it be in 2 hours?"
        Ok(ToolResult::Escalation(serde_json::json!({
            "current_time": time_str,
            "timestamp": now.timestamp(),
            "hour": now.hour(),
            "minute": now.minute(),
            "period": now.format("%p").to_string()
        })))
    }
}

/// Calculate future time based on current time and offset
pub async fn calculate_future_time(
    arguments: Value,
    cancel_token: CancellationToken,
) -> Result<ToolResult, ToolError> {
    // Check for cancellation
    if cancel_token.is_cancelled() {
        return Err(ToolError::Cancelled);
    }

    // Extract hours and minutes from arguments
    let hours = arguments.get("hours").and_then(|v| v.as_i64()).unwrap_or(0);
    let minutes = arguments
        .get("minutes")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    let now: DateTime<Local> = Local::now();
    let future_time = now + Duration::hours(hours) + Duration::minutes(minutes);

    // Format time in a natural, conversational way
    let time_str = future_time.format("%I:%M %p").to_string();
    let time_str = time_str.trim_start_matches('0'); // Remove leading zero from hour

    // Create a natural response
    let mut response = String::new();
    if hours > 0 && minutes > 0 {
        response = format!(
            "In {} hours and {} minutes, it will be {}",
            hours, minutes, time_str
        );
    } else if hours > 0 {
        response = format!("In {} hours, it will be {}", hours, time_str);
    } else if minutes > 0 {
        response = format!("In {} minutes, it will be {}", minutes, time_str);
    }

    Ok(ToolResult::Success(Some(response)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn test_get_time() {
        let cancel_token = CancellationToken::new();
        let result = get_time(json!({"send_output_directly_to_tts": true}), cancel_token).await;
        assert!(result.is_ok());

        if let Ok(ToolResult::Success(Some(time_str))) = result {
            assert!(time_str.starts_with("It's "));
            assert!(time_str.len() > 5); // Should be longer than just "It's "
        } else {
            panic!("Expected Success result");
        }
    }

    #[tokio::test]
    async fn test_get_time_format() {
        let cancel_token = CancellationToken::new();
        let result = get_time(json!({"send_output_directly_to_tts": true}), cancel_token)
            .await
            .unwrap();

        if let ToolResult::Success(Some(time_str)) = result {
            // Should contain "It's" and a time format like "3:45 PM"
            assert!(time_str.contains("It's"));
            assert!(time_str.contains(":"));
            assert!(time_str.contains("AM") || time_str.contains("PM"));
        } else {
            panic!("Expected Success result");
        }
    }

    #[tokio::test]
    async fn test_get_time_cancellation() {
        // Test cancellation support
        let cancel_token = CancellationToken::new();
        cancel_token.cancel(); // Cancel immediately

        let result = get_time(json!({"send_output_directly_to_tts": true}), cancel_token).await;
        assert!(result.is_err());

        if let Err(ToolError::Cancelled) = result {
            // Expected cancellation error
        } else {
            panic!("Expected Cancelled error, got: {:?}", result);
        }
    }

    #[tokio::test]
    async fn test_get_time_routing_parameter() {
        let cancel_token = CancellationToken::new();

        // Test with send_output_directly_to_tts = true (should return response for TTS)
        let result_direct = get_time(
            json!({"send_output_directly_to_tts": true}),
            cancel_token.clone(),
        )
        .await;
        assert!(result_direct.is_ok());
        if let Ok(ToolResult::Success(Some(msg))) = result_direct {
            assert!(msg.starts_with("It's "));
            assert!(msg.contains(":"));
            assert!(msg.contains("AM") || msg.contains("PM"));
        } else {
            panic!("Expected Success(Some(_)) for direct output");
        }

        // Test with send_output_directly_to_tts = false (should return data for LLM processing)
        let result_llm = get_time(
            json!({"send_output_directly_to_tts": false}),
            cancel_token.clone(),
        )
        .await;
        assert!(result_llm.is_ok());
        if let Ok(ToolResult::Escalation(data)) = result_llm {
            assert!(data.get("current_time").is_some());
            assert!(data.get("timestamp").is_some());
            assert!(data.get("hour").is_some());
            assert!(data.get("minute").is_some());
            assert!(data.get("period").is_some());
        } else {
            panic!("Expected Escalation for LLM processing");
        }
    }
}
