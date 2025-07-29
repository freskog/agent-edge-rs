use crate::error::AgentError;
use crate::services::types::{LLMResponse, ToolRegistry};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

impl Message {
    pub fn system(content: &str) -> Self {
        Self {
            role: "system".to_string(),
            content: content.to_string(),
        }
    }

    pub fn user(content: &str) -> Self {
        Self {
            role: "user".to_string(),
            content: content.to_string(),
        }
    }

    pub fn assistant(content: &str) -> Self {
        Self {
            role: "assistant".to_string(),
            content: content.to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct GroqLLM {
    api_key: String,
    base_url: String,
}

impl GroqLLM {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            base_url: "https://api.groq.com/openai/v1".to_string(),
        }
    }

    /// Complete with tools (now blocking using ureq)
    pub fn complete_with_tools(
        &self,
        messages: Vec<Message>,
        tool_registry: &ToolRegistry,
    ) -> Result<LLMResponse, AgentError> {
        let request_body = serde_json::json!({
            "model": "llama-3.1-8b-instant",
            "messages": messages,
            "tools": tool_registry.get_tool_definitions(),
            "tool_choice": "auto",
            "temperature": 0.7,
            "max_tokens": 1024
        });

        let response = ureq::post(&format!("{}/chat/completions", self.base_url))
            .set("Authorization", &format!("Bearer {}", self.api_key))
            .set("Content-Type", "application/json")
            .send_json(&request_body)
            .map_err(|e| AgentError::LLM(format!("HTTP request failed: {}", e)))?;

        let response_json: serde_json::Value = response
            .into_json()
            .map_err(|e| AgentError::LLM(format!("Failed to parse JSON response: {}", e)))?;

        // Parse tool calls from response
        let tool_calls = self.parse_tool_calls(&response_json)?;

        Ok(LLMResponse { tool_calls })
    }

    fn parse_tool_calls(
        &self,
        response: &serde_json::Value,
    ) -> Result<Vec<crate::services::types::ToolCall>, AgentError> {
        let mut tool_calls = Vec::new();

        if let Some(choices) = response["choices"].as_array() {
            if let Some(first_choice) = choices.first() {
                if let Some(message) = first_choice["message"].as_object() {
                    if let Some(calls) = message["tool_calls"].as_array() {
                        for call in calls {
                            if let (Some(name), Some(args)) = (
                                call["function"]["name"].as_str(),
                                call["function"]["arguments"].as_str(),
                            ) {
                                // Parse arguments JSON
                                let arguments: HashMap<String, serde_json::Value> =
                                    serde_json::from_str(args).unwrap_or_else(|_| HashMap::new());

                                // Convert to our simplified format
                                if name == "respond" {
                                    if let Some(text) =
                                        arguments.get("message").and_then(|v| v.as_str())
                                    {
                                        tool_calls.push(crate::services::types::ToolCall {
                                            name: "respond".to_string(),
                                            text: text.to_string(),
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(tool_calls)
    }
}

// Remove the problematic async trait implementations since we're now fully blocking

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_creation() {
        let system_msg = Message::system("You are a helpful assistant");
        assert_eq!(system_msg.role, "system");
        assert_eq!(system_msg.content, "You are a helpful assistant");

        let user_msg = Message::user("Hello");
        assert_eq!(user_msg.role, "user");
        assert_eq!(user_msg.content, "Hello");

        let assistant_msg = Message::assistant("Hi there!");
        assert_eq!(assistant_msg.role, "assistant");
        assert_eq!(assistant_msg.content, "Hi there!");
    }
}
