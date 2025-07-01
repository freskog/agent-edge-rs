use super::client::{GroqLLM, Message, ToolCall};
use super::context::ConversationContext;
use super::prompts::SystemPrompts;
use super::tools::{ToolError, ToolRegistry, ToolResult};
use crate::config::ApiConfig;
use serde_json::Value;
use thiserror::Error;
use tokio_util::sync::CancellationToken;

#[derive(Error, Debug)]
pub enum IntegrationError {
    #[error("LLM error: {0}")]
    LLM(#[from] super::client::LLMError),
    #[error("Tool error: {0}")]
    Tool(#[from] ToolError),
    #[error("Configuration error: {0}")]
    Config(String),
    #[error("Operation was cancelled")]
    Cancelled,
}

pub struct LLMIntegration {
    llm: GroqLLM,
    tool_registry: ToolRegistry,
    context: ConversationContext,
}

impl LLMIntegration {
    pub fn new(config: &ApiConfig) -> Result<Self, IntegrationError> {
        let llm = GroqLLM::new(config.groq_key().to_string());
        let tool_registry = super::tools::create_default_registry();
        let mut context = ConversationContext::with_defaults();

        // Set the tool-aware system prompt
        context.set_system_message(SystemPrompts::tool_aware_voice_assistant());

        Ok(Self {
            llm,
            tool_registry,
            context,
        })
    }

    /// Process user input and return a response with cancellation support
    pub async fn process_user_instruction(
        &mut self,
        user_input: &str,
        cancel_token: CancellationToken,
    ) -> Result<Option<String>, IntegrationError> {
        // Check if already cancelled
        if cancel_token.is_cancelled() {
            return Err(IntegrationError::Cancelled);
        }

        // Empty transcript = pure abort (silent cancellation)
        if user_input.trim().is_empty() {
            log::info!("Empty transcript received, aborting silently");
            return Ok(None);
        }

        // Add user message to context
        self.context.add_user_message(user_input);

        // Get all messages for LLM
        let messages = self.context.get_messages();

        // Get tool definitions
        let tools = self.tool_registry.get_tool_definitions();

        // Call LLM with tools using internal config
        let response = tokio::select! {
            result = self.llm.complete_with_internal_tools(messages, &tools) => {
                result?
            }
            _ = cancel_token.cancelled() => {
                return Err(IntegrationError::Cancelled);
            }
        };

        // Process tool calls if any
        if !response.tool_calls.is_empty() {
            return self
                .process_tool_calls(&response.tool_calls, cancel_token)
                .await;
        }

        // No tools called - add assistant response to context and return
        self.context.add_assistant_message(&response.content);
        Ok(Some(response.content))
    }

    /// Process tool calls and return the final response
    async fn process_tool_calls(
        &mut self,
        tool_calls: &[ToolCall],
        cancel_token: CancellationToken,
    ) -> Result<Option<String>, IntegrationError> {
        // For now, process the first tool call
        // TODO: Handle multiple tool calls if needed
        if let Some(tool_call) = tool_calls.first() {
            let result = tokio::select! {
                result = self.tool_registry.execute_tool(
                    &tool_call.name,
                    tool_call.arguments.clone(),
                    cancel_token.clone()
                ) => {
                    result?
                }
                _ = cancel_token.cancelled() => {
                    return Err(IntegrationError::Cancelled);
                }
            };

            return match result {
                ToolResult::Success(message) => {
                    // Add assistant response to context if there's a message
                    if let Some(ref content) = message {
                        self.context.add_assistant_message(content);
                    }
                    Ok(message)
                }
                ToolResult::Escalation(data) => {
                    // Send tool result back to LLM for processing
                    self.process_tool_escalation(tool_call, data, cancel_token)
                        .await
                }
            };
        }

        Err(IntegrationError::Config(
            "No tool calls to process".to_string(),
        ))
    }

    /// Process tool escalation by sending result back to LLM
    async fn process_tool_escalation(
        &mut self,
        tool_call: &ToolCall,
        data: Value,
        cancel_token: CancellationToken,
    ) -> Result<Option<String>, IntegrationError> {
        // Check for cancellation
        if cancel_token.is_cancelled() {
            return Err(IntegrationError::Cancelled);
        }

        // Create a message explaining the tool result
        let tool_result_message = format!(
            "Tool '{}' returned the following result that needs processing: {}",
            tool_call.name,
            serde_json::to_string_pretty(&data).unwrap_or_else(|_| "invalid json".to_string())
        );

        // Add the tool result as a system message
        let mut messages = vec![Message::system(&tool_result_message)];
        messages.extend(self.context.get_messages());

        // Get tool definitions for potential follow-up tool calls
        let tools = self.tool_registry.get_tool_definitions();

        // Ask LLM to process the tool result
        let response = tokio::select! {
            result = self.llm.complete_with_internal_tools(messages, &tools) => {
                result?
            }
            _ = cancel_token.cancelled() => {
                return Err(IntegrationError::Cancelled);
            }
        };

        // Add assistant response to context
        self.context.add_assistant_message(&response.content);
        Ok(Some(response.content))
    }

    /// Get conversation context summary for debugging
    pub fn context_summary(&self) -> String {
        self.context.summary()
    }

    /// Clear conversation context
    pub fn clear_context(&mut self) {
        self.context.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ApiConfig;

    // Mock API config for testing
    fn create_mock_config() -> ApiConfig {
        ApiConfig {
            fireworks_key: secrecy::SecretBox::new(Box::new("mock_fireworks_key".to_string())),
            groq_key: secrecy::SecretBox::new(Box::new("mock_groq_key".to_string())),
            elevenlabs_key: secrecy::SecretBox::new(Box::new("mock_elevenlabs_key".to_string())),
        }
    }

    #[test]
    fn test_integration_creation() {
        let config = create_mock_config();
        let integration = LLMIntegration::new(&config);
        assert!(integration.is_ok());
    }

    #[test]
    fn test_context_summary() {
        let config = create_mock_config();
        let integration = LLMIntegration::new(&config).unwrap();
        let summary = integration.context_summary();
        assert!(summary.contains("Context:"));
    }

    #[test]
    fn test_clear_context() {
        let config = create_mock_config();
        let mut integration = LLMIntegration::new(&config).unwrap();
        integration.clear_context();
        assert!(integration.context.is_empty());
    }
}
