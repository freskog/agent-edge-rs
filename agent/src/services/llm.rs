use crate::config::ApiConfig;
use crate::error::AgentError;
use crate::llm::client::GroqLLM;
use crate::services::types::{create_default_registry, LLMResponse, ToolRegistry};
use crate::services::LLMService;

/// Groq LLM service implementation (now blocking)
pub struct GroqLLMService {
    llm: GroqLLM,
    tool_registry: ToolRegistry,
}

impl GroqLLMService {
    pub fn new(config: &ApiConfig) -> Result<Self, AgentError> {
        let llm = GroqLLM::new(config.groq_key().to_string());
        let tool_registry = create_default_registry();

        Ok(Self { llm, tool_registry })
    }
}

impl LLMService for GroqLLMService {
    /// Process user transcript and return tool calls (now blocking)
    fn process(&self, transcript: String) -> Result<LLMResponse, AgentError> {
        log::info!("🤖 Processing transcript with LLM: '{}'", transcript);

        let messages = vec![
            crate::llm::client::Message::system(
                "You are a helpful voice assistant. Use the available tools to respond to the user."
            ),
            crate::llm::client::Message::user(&transcript),
        ];

        // Call LLM with tools (now blocking)
        let llm_response = self
            .llm
            .complete_with_tools(messages, &self.tool_registry)
            .map_err(|e| AgentError::LLM(format!("LLM request failed: {}", e)))?;

        log::info!(
            "🤖 LLM response received with {} tool calls",
            llm_response.tool_calls.len()
        );

        Ok(llm_response)
    }
}
