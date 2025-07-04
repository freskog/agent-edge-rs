use reqwest::Client;
use serde_json::{json, Value};
use std::time::Duration;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum LLMError {
    #[error("HTTP request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("API error: {status} - {message}")]
    ApiError { status: u16, message: String },
    #[error("Response parsing error: {0}")]
    ParseError(String),
    #[error("Configuration error: {0}")]
    Config(String),
}

#[derive(Debug, Clone)]
pub struct LLMConfig {
    pub model: String,
    pub temperature: f32,
    pub max_tokens: Option<u32>,
    pub top_p: f32,
    pub frequency_penalty: f32,
    pub presence_penalty: f32,
}

impl Default for LLMConfig {
    fn default() -> Self {
        Self {
            model: "meta-llama/llama-4-maverick-17b-128e-instruct".to_string(),
            temperature: 0.3,
            max_tokens: Some(8192),
            top_p: 1.0,
            frequency_penalty: 0.0,
            presence_penalty: 0.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Message {
    pub role: String,
    pub content: String,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".to_string(),
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".to_string(),
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant".to_string(),
            content: content.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug)]
pub struct LLMResponse {
    pub content: String,
    pub usage: Option<Usage>,
    pub model: String,
    pub finish_reason: Option<String>,
    pub tool_calls: Vec<ToolCall>,
}

#[derive(Debug)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

pub struct GroqLLM {
    client: Client,
    api_key: String,
    base_url: String,
    config: LLMConfig,
}

impl GroqLLM {
    pub fn new(api_key: String) -> Self {
        Self::with_config(api_key, LLMConfig::default())
    }

    pub fn with_config(api_key: String, config: LLMConfig) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(60)) // LLM calls can be slow
            .build()
            .expect("Failed to create HTTP client");

        Self {
            client,
            api_key,
            base_url: "https://api.groq.com/openai/v1".to_string(),
            config,
        }
    }

    /// Generate a completion from messages
    pub async fn complete(&self, messages: Vec<Message>) -> Result<LLMResponse, LLMError> {
        self.complete_with_config(messages, &self.config).await
    }

    /// Generate a completion with tools using the internal config
    pub async fn complete_with_internal_tools(
        &self,
        messages: Vec<Message>,
        tools: &[Value],
    ) -> Result<LLMResponse, LLMError> {
        self.complete_with_tools(messages, &self.config, tools)
            .await
    }

    /// Generate a completion with custom config
    pub async fn complete_with_config(
        &self,
        messages: Vec<Message>,
        config: &LLMConfig,
    ) -> Result<LLMResponse, LLMError> {
        self.complete_with_tools(messages, config, &[]).await
    }

    /// Generate a completion with tools
    pub async fn complete_with_tools(
        &self,
        messages: Vec<Message>,
        config: &LLMConfig,
        tools: &[Value],
    ) -> Result<LLMResponse, LLMError> {
        let url = format!("{}/chat/completions", self.base_url);

        let messages_json: Vec<Value> = messages
            .into_iter()
            .map(|msg| {
                json!({
                    "role": msg.role,
                    "content": msg.content
                })
            })
            .collect();

        let mut payload = json!({
            "model": config.model,
            "messages": messages_json,
            "temperature": config.temperature,
            "top_p": config.top_p,
            "frequency_penalty": config.frequency_penalty,
            "presence_penalty": config.presence_penalty,
            "stream": false
        });

        if let Some(max_tokens) = config.max_tokens {
            payload["max_tokens"] = json!(max_tokens);
        }

        // Add tools if provided
        if !tools.is_empty() {
            payload["tools"] = json!(tools);
            payload["tool_choice"] = json!("auto");
        }

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await?;

        let status = response.status();

        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(LLMError::ApiError {
                status: status.as_u16(),
                message: error_text,
            });
        }

        let response_text = response.text().await?;
        self.parse_response(&response_text)
    }

    /// Generate a streaming completion (returns first chunk for now, full streaming support would require async streams)
    pub async fn complete_streaming(
        &self,
        messages: Vec<Message>,
    ) -> Result<LLMResponse, LLMError> {
        let url = format!("{}/chat/completions", self.base_url);

        let messages_json: Vec<Value> = messages
            .into_iter()
            .map(|msg| {
                json!({
                    "role": msg.role,
                    "content": msg.content
                })
            })
            .collect();

        let mut payload = json!({
            "model": self.config.model,
            "messages": messages_json,
            "temperature": self.config.temperature,
            "top_p": self.config.top_p,
            "frequency_penalty": self.config.frequency_penalty,
            "presence_penalty": self.config.presence_penalty,
            "stream": true
        });

        if let Some(max_tokens) = self.config.max_tokens {
            payload["max_tokens"] = json!(max_tokens);
        }

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await?;

        let status = response.status();

        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(LLMError::ApiError {
                status: status.as_u16(),
                message: error_text,
            });
        }

        // For now, collect all streaming chunks and return the complete response
        // TODO: Implement proper streaming with async streams
        let response_text = response.text().await?;
        self.parse_streaming_response(&response_text)
    }

    /// Parse the JSON response from Groq API
    fn parse_response(&self, response_text: &str) -> Result<LLMResponse, LLMError> {
        let json: Value = serde_json::from_str(response_text)
            .map_err(|e| LLMError::ParseError(format!("Invalid JSON: {}", e)))?;

        let choices = json["choices"]
            .as_array()
            .ok_or_else(|| LLMError::ParseError("Missing 'choices' field".to_string()))?;

        if choices.is_empty() {
            return Err(LLMError::ParseError("Empty choices array".to_string()));
        }

        let first_choice = &choices[0];
        let message = &first_choice["message"];

        let content = message["content"]
            .as_str()
            .unwrap_or("") // Content can be null when tool calls are made
            .to_string();

        let finish_reason = first_choice["finish_reason"]
            .as_str()
            .map(|s| s.to_string());

        let model = json["model"]
            .as_str()
            .unwrap_or(&self.config.model)
            .to_string();

        let usage = if let Some(usage_json) = json.get("usage") {
            Some(Usage {
                prompt_tokens: usage_json["prompt_tokens"].as_u64().unwrap_or(0) as u32,
                completion_tokens: usage_json["completion_tokens"].as_u64().unwrap_or(0) as u32,
                total_tokens: usage_json["total_tokens"].as_u64().unwrap_or(0) as u32,
            })
        } else {
            None
        };

        // Parse tool calls if present
        let mut tool_calls = Vec::new();
        if let Some(tool_calls_json) = message.get("tool_calls") {
            if let Some(tool_calls_array) = tool_calls_json.as_array() {
                for tool_call_json in tool_calls_array {
                    if let (Some(id), Some(name), Some(function)) = (
                        tool_call_json["id"].as_str(),
                        tool_call_json["function"]["name"].as_str(),
                        tool_call_json["function"]["arguments"].as_str(),
                    ) {
                        let arguments: Value = serde_json::from_str(function).map_err(|e| {
                            LLMError::ParseError(format!("Invalid tool call arguments: {}", e))
                        })?;

                        tool_calls.push(ToolCall {
                            id: id.to_string(),
                            name: name.to_string(),
                            arguments,
                        });
                    }
                }
            }
        }

        Ok(LLMResponse {
            content,
            usage,
            model,
            finish_reason,
            tool_calls,
        })
    }

    /// Parse streaming response (simplified - collects all chunks)
    fn parse_streaming_response(&self, response_text: &str) -> Result<LLMResponse, LLMError> {
        let mut complete_content = String::new();
        let mut model = self.config.model.clone();
        let mut finish_reason = None;

        // Parse SSE format
        for line in response_text.lines() {
            if line.starts_with("data: ") {
                let data = &line[6..]; // Remove "data: " prefix

                if data == "[DONE]" {
                    break;
                }

                if let Ok(json) = serde_json::from_str::<Value>(data) {
                    if let Some(choices) = json["choices"].as_array() {
                        if let Some(first_choice) = choices.first() {
                            if let Some(delta) = first_choice.get("delta") {
                                if let Some(content) = delta["content"].as_str() {
                                    complete_content.push_str(content);
                                }
                            }

                            if let Some(reason) = first_choice["finish_reason"].as_str() {
                                finish_reason = Some(reason.to_string());
                            }
                        }
                    }

                    if let Some(model_name) = json["model"].as_str() {
                        model = model_name.to_string();
                    }
                }
            }
        }

        Ok(LLMResponse {
            content: complete_content,
            usage: None, // Usage not typically provided in streaming
            model,
            finish_reason,
            tool_calls: Vec::new(), // Tool calls not supported in streaming for now
        })
    }

    /// Simple convenience method for single-shot completions
    pub async fn ask(&self, prompt: &str) -> Result<String, LLMError> {
        let messages = vec![Message::user(prompt)];
        let response = self.complete(messages).await?;
        Ok(response.content)
    }

    /// Ask with system message
    pub async fn ask_with_system(
        &self,
        system_prompt: &str,
        user_prompt: &str,
    ) -> Result<String, LLMError> {
        let messages = vec![Message::system(system_prompt), Message::user(user_prompt)];
        let response = self.complete(messages).await?;
        Ok(response.content)
    }

    /// Get available models
    pub async fn get_models(&self) -> Result<Vec<String>, LLMError> {
        let url = format!("{}/models", self.base_url);

        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await?;

        let status = response.status();

        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(LLMError::ApiError {
                status: status.as_u16(),
                message: error_text,
            });
        }

        let response_text = response.text().await?;
        let json: Value = serde_json::from_str(&response_text)
            .map_err(|e| LLMError::ParseError(format!("Invalid JSON: {}", e)))?;

        let models = json["data"]
            .as_array()
            .ok_or_else(|| LLMError::ParseError("Missing 'data' field".to_string()))?
            .iter()
            .filter_map(|model| model["id"].as_str().map(|s| s.to_string()))
            .collect();

        Ok(models)
    }
}

#[async_trait::async_trait]
pub trait LLMClient: Send + Sync {
    async fn complete_with_internal_tools(
        &self,
        messages: Vec<Message>,
        tools: &[serde_json::Value],
    ) -> Result<LLMResponse, LLMError>;
}

#[async_trait::async_trait]
impl LLMClient for GroqLLM {
    async fn complete_with_internal_tools(
        &self,
        messages: Vec<Message>,
        tools: &[serde_json::Value],
    ) -> Result<LLMResponse, LLMError> {
        self.complete_with_internal_tools(messages, tools).await
    }
}

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

    #[test]
    fn test_config_defaults() {
        let config = LLMConfig::default();
        assert_eq!(
            config.model,
            "meta-llama/llama-4-maverick-17b-128e-instruct"
        );
        assert_eq!(config.temperature, 0.3);
        assert_eq!(config.max_tokens, Some(8192));
        assert_eq!(config.top_p, 1.0);
        assert_eq!(config.frequency_penalty, 0.0);
        assert_eq!(config.presence_penalty, 0.0);
    }
}
