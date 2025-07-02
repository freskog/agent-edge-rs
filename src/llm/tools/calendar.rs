use super::{ToolError, ToolResult};
use chrono::{DateTime, Local};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

/// Get the current time in a human-readable format
pub async fn get_current_time(
    _arguments: Value,
    cancel_token: CancellationToken,
) -> Result<ToolResult, ToolError> {
    // Check for cancellation
    if cancel_token.is_cancelled() {
        return Err(ToolError::Cancelled);
    }

    let now: DateTime<Local> = Local::now();
    let time_str = now.format("%I:%M %p").to_string();
    let time_str = time_str.trim_start_matches('0'); // Remove leading zero from hour

    // Return success with time data for LLM processing
    Ok(ToolResult::Response(format!("Current time: {}", time_str)))
}

/// Get the current date in a human-readable format
pub async fn get_current_date(
    _arguments: Value,
    cancel_token: CancellationToken,
) -> Result<ToolResult, ToolError> {
    // Check for cancellation
    if cancel_token.is_cancelled() {
        return Err(ToolError::Cancelled);
    }

    let now: DateTime<Local> = Local::now();
    let date_str = now.format("%A, %B %d, %Y").to_string();

    // Return success with date data for LLM processing
    Ok(ToolResult::Response(format!("Current date: {}", date_str)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn test_get_current_time() {
        let cancel_token = CancellationToken::new();
        let result = get_current_time(json!({}), cancel_token).await;
        assert!(result.is_ok());

        if let Ok(ToolResult::Response(data)) = result {
            assert!(!data.is_empty());
        } else {
            panic!("Expected Response result");
        }
    }

    #[tokio::test]
    async fn test_get_current_date() {
        let cancel_token = CancellationToken::new();
        let result = get_current_date(json!({}), cancel_token).await;
        assert!(result.is_ok());

        if let Ok(ToolResult::Response(data)) = result {
            assert!(!data.is_empty());
        } else {
            panic!("Expected Response result");
        }
    }

    #[tokio::test]
    async fn test_get_current_time_cancellation() {
        let cancel_token = CancellationToken::new();
        cancel_token.cancel(); // Cancel immediately

        let result = get_current_time(json!({}), cancel_token).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ToolError::Cancelled));
    }

    #[tokio::test]
    async fn test_get_current_date_cancellation() {
        let cancel_token = CancellationToken::new();
        cancel_token.cancel(); // Cancel immediately

        let result = get_current_date(json!({}), cancel_token).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ToolError::Cancelled));
    }
}
