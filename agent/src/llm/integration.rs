use super::client::{GroqLLM, Message, ToolCall};
use super::context::ConversationContext;
use super::prompts::SystemPrompts;
use super::tools::{ToolError, ToolRegistry};
use crate::config::ApiConfig;
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

        // Clear context for each new interaction to ensure clean state
        self.context.clear();
        log::debug!("🔄 Context reset for new interaction");

        log::info!("🔄 Processing user instruction: '{}'", user_input);

        // Add user message to context
        self.context.add_user_message(user_input);

        // Get all messages for LLM
        let messages = self.context.get_messages();
        log::debug!("📝 Context has {} messages", messages.len());

        // Get tool definitions
        let tools = self.tool_registry.get_tool_definitions();
        log::debug!("🔧 Available tools: {}", tools.len());
        for tool in &tools {
            if let Some(function) = tool.get("function") {
                if let Some(name) = function.get("name") {
                    log::debug!("  - {}", name.as_str().unwrap_or("unknown"));
                }
            }
        }

        // Call LLM with tools using internal config
        log::info!("🤖 Calling LLM with {} tools...", tools.len());
        let response = tokio::select! {
            result = self.llm.complete_with_internal_tools(messages, &tools) => {
                result?
            }
            _ = cancel_token.cancelled() => {
                return Err(IntegrationError::Cancelled);
            }
        };

        log::info!("📨 LLM Response received:");
        log::info!("  - Content: '{}'", response.content);
        log::info!("  - Content length: {} chars", response.content.len());
        log::info!("  - Tool calls: {}", response.tool_calls.len());
        log::info!("  - Finish reason: {:?}", response.finish_reason);

        for (i, tool_call) in response.tool_calls.iter().enumerate() {
            log::info!(
                "  - Tool call {}: {} with args: {}",
                i,
                tool_call.name,
                tool_call.arguments
            );
        }

        // Process tool calls if any
        if !response.tool_calls.is_empty() {
            log::info!("🔧 Processing {} tool calls", response.tool_calls.len());
            return self
                .process_tool_calls(&response.tool_calls, cancel_token)
                .await;
        }

        // No tools called - add assistant response to context and return
        log::info!(
            "🤔 No tool calls – treating assistant content as internal reasoning (not spoken)"
        );
        self.context.add_assistant_message(&response.content);

        if response.content.is_empty() {
            log::warn!("⚠️  LLM returned empty content with no tool calls!");
        }

        // Internal content, nothing to speak
        Ok(None)
    }

    /// Process tool calls and return the final response
    async fn process_tool_calls(
        &mut self,
        tool_calls: &[ToolCall],
        cancel_token: CancellationToken,
    ) -> Result<Option<String>, IntegrationError> {
        if tool_calls.is_empty() {
            return Err(IntegrationError::Config(
                "No tool calls to process".to_string(),
            ));
        }

        log::info!("🔧 Executing {} tool calls in parallel", tool_calls.len());

        // Execute all tool calls in parallel
        let mut futures = Vec::new();
        for tool_call in tool_calls {
            log::info!(
                "🔧 Queuing tool: '{}' with args: {}",
                tool_call.name,
                tool_call.arguments
            );

            let future = self.tool_registry.execute_tool(
                &tool_call.name,
                tool_call.arguments.clone(),
                cancel_token.clone(),
            );
            futures.push((tool_call.clone(), future));
        }

        // Wait for all tools to complete
        let mut results = Vec::new();
        for (tool_call, future) in futures {
            let result = tokio::select! {
                result = future => {
                    result?
                }
                _ = cancel_token.cancelled() => {
                    return Err(IntegrationError::Cancelled);
                }
            };

            log::info!(
                "🔧 Tool '{}' execution result: {:?}",
                tool_call.name,
                match &result {
                    crate::llm::tools::ToolResult::Ok => "Ok".to_string(),
                    crate::llm::tools::ToolResult::Response(msg) => format!("Response('{}')", msg),
                    crate::llm::tools::ToolResult::Escalation(_) => "Escalation(...)".to_string(),
                }
            );

            results.push((tool_call, result));
        }

        // Check if tell_user was called - this ends the conversation
        for (tool_call, result) in &results {
            if tool_call.name == "tell_user" {
                if let crate::llm::tools::ToolResult::Response(message) = result {
                    log::info!(
                        "💬 tell_user called, ending conversation with: '{}'",
                        message
                    );
                    self.context.add_assistant_message(message);
                    return Ok(Some(message.clone()));
                }
            }
        }

        // No tell_user called - send all results back to LLM for processing
        log::info!("🔄 No tell_user called, returning results to LLM for processing");
        self.process_tool_results_for_llm(results, cancel_token)
            .await
    }

    /// Process tool results by sending them back to LLM for formulation
    async fn process_tool_results_for_llm(
        &mut self,
        results: Vec<(ToolCall, crate::llm::tools::ToolResult)>,
        cancel_token: CancellationToken,
    ) -> Result<Option<String>, IntegrationError> {
        // Check for cancellation
        if cancel_token.is_cancelled() {
            return Err(IntegrationError::Cancelled);
        }

        log::info!("🔄 Processing {} tool results for LLM", results.len());

        // Create messages explaining all tool results
        let mut result_messages = Vec::new();
        for (tool_call, result) in &results {
            match result {
                crate::llm::tools::ToolResult::Ok => {
                    let message = format!("Tool '{}' completed successfully", tool_call.name);
                    result_messages.push(message);
                }
                crate::llm::tools::ToolResult::Response(msg) => {
                    let message = format!("Tool '{}' returned: {}", tool_call.name, msg);
                    result_messages.push(message);
                }
                crate::llm::tools::ToolResult::Escalation(reason) => {
                    let message = format!(
                        "Tool '{}' failed or cannot fulfill goal: {}",
                        tool_call.name, reason
                    );
                    result_messages.push(message);
                }
            }
        }

        let combined_results = result_messages.join("\n\n");
        log::debug!("📝 Combined tool results for LLM: {}", combined_results);

        // Get the original messages for context
        let mut messages = self.context.get_messages();

        // Find the original user question from the context
        let original_question = messages
            .iter()
            .find(|msg| msg.role == "user")
            .map_or("unknown question".to_string(), |msg| msg.content.clone());

        // Add a system message explaining what to do with the tool results
        messages.insert(0, Message::system(&format!(
            "The user asked: \"{}\"\n\n\
            You have received the following tool results:\n\n{}\n\n\
            INSTRUCTIONS:\n\
            1. Use these tool results to answer the user's question: \"{}\"\n\
            2. ALWAYS use the tell_user tool to provide your final response\n\
            3. NEVER return raw tool results or make unnecessary tool calls\n\
            4. If you need to calculate something (like time differences), do the math yourself\n\
            5. Format your response naturally for speech\n\
            6. DO NOT call the same tool again - use the data you already have",
            original_question, combined_results, original_question
        )));

        // Get tool definitions for potential follow-up tool calls
        let tools = self.tool_registry.get_tool_definitions();

        log::info!("🤖 Calling LLM to process tool results...");
        // Ask LLM to process the tool results
        let response = tokio::select! {
            result = self.llm.complete_with_internal_tools(messages, &tools) => {
                result?
            }
            _ = cancel_token.cancelled() => {
                return Err(IntegrationError::Cancelled);
            }
        };

        log::info!("📨 LLM Tool Processing Response:");
        log::info!("  - Content: '{}'", response.content);
        log::info!("  - Content length: {} chars", response.content.len());
        log::info!("  - Tool calls: {}", response.tool_calls.len());

        // If LLM made more tool calls, process them recursively
        if !response.tool_calls.is_empty() {
            log::info!(
                "🔄 LLM made {} additional tool calls",
                response.tool_calls.len()
            );
            return Box::pin(self.process_tool_calls(&response.tool_calls, cancel_token)).await;
        }

        // No more tool calls - add response to context and return
        if !response.content.is_empty() {
            self.context.add_assistant_message(&response.content);
            Ok(Some(response.content))
        } else {
            log::warn!("⚠️  LLM returned empty response after tool processing");
            Ok(None)
        }
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
