use serde_json::Value;
use thiserror::Error;
use tokio_util::sync::CancellationToken;

pub mod quick_actions;

#[derive(Error, Debug)]
pub enum ToolError {
    #[error("Tool not found: {0}")]
    NotFound(String),
    #[error("Tool execution failed: {0}")]
    ExecutionFailed(String),
    #[error("Invalid tool parameters: {0}")]
    InvalidParameters(String),
    #[error("Tool timeout: {0}")]
    Timeout(String),
    #[error("Tool execution was cancelled")]
    Cancelled,
}

#[derive(Debug)]
pub enum ToolResult {
    Success(Option<String>), // Happy path: Some(msg) = speak it, None = silent
    Escalation(Value),       // Tool needs LLM help/intervention
}

#[derive(Debug, Clone)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

pub struct ToolRegistry {
    tools: Vec<Tool>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self { tools: Vec::new() }
    }

    /// Register a tool
    pub fn register_tool(&mut self, tool: Tool) {
        self.tools.push(tool);
    }

    /// Get available tools
    pub fn get_tools(&self) -> &[Tool] {
        &self.tools
    }

    /// Find a tool by name
    pub fn find_tool(&self, name: &str) -> Option<&Tool> {
        self.tools.iter().find(|tool| tool.name == name)
    }

    /// Execute a tool by name with cancellation support
    pub async fn execute_tool(
        &self,
        name: &str,
        arguments: Value,
        cancel_token: CancellationToken,
    ) -> Result<ToolResult, ToolError> {
        // Check if already cancelled before starting
        if cancel_token.is_cancelled() {
            return Err(ToolError::Cancelled);
        }

        match name {
            "get_time" => quick_actions::get_time(arguments, cancel_token).await,
            "calculate_future_time" => {
                quick_actions::calculate_future_time(arguments, cancel_token).await
            }
            _ => Err(ToolError::NotFound(format!("Tool '{}' not found", name))),
        }
    }

    /// Get tool definitions for LLM function calling
    pub fn get_tool_definitions(&self) -> Vec<Value> {
        self.tools
            .iter()
            .map(|tool| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.parameters
                    }
                })
            })
            .collect()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Initialize the default tool registry with all available tools
pub fn create_default_registry() -> ToolRegistry {
    let mut registry = ToolRegistry::new();

    // Register quick action tools
    registry.register_tool(Tool {
        name: "get_time".to_string(),
        description: "Get the current time in a human-readable format".to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "send_output_directly_to_tts": {
                    "type": "boolean",
                    "description": "true = send output directly to speech, false = return data for LLM processing"
                }
            },
            "required": ["send_output_directly_to_tts"]
        }),
    });

    registry.register_tool(Tool {
        name: "calculate_future_time".to_string(),
        description:
            "Calculate what time it will be in a specified number of hours and/or minutes from now"
                .to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "hours": {
                    "type": "integer",
                    "description": "Number of hours to add to current time"
                },
                "minutes": {
                    "type": "integer",
                    "description": "Number of minutes to add to current time"
                }
            },
            "required": ["hours", "minutes"]
        }),
    });

    registry
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_tool_registry_creation() {
        let registry = ToolRegistry::new();
        assert_eq!(registry.get_tools().len(), 0);
    }

    #[test]
    fn test_tool_registration() {
        let mut registry = ToolRegistry::new();

        let tool = Tool {
            name: "test_tool".to_string(),
            description: "A test tool".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "input": {
                        "type": "string",
                        "description": "Test input"
                    }
                },
                "required": ["input"]
            }),
        };

        registry.register_tool(tool);
        assert_eq!(registry.get_tools().len(), 1);
        assert_eq!(registry.get_tools()[0].name, "test_tool");
    }

    #[test]
    fn test_tool_definitions() {
        let mut registry = ToolRegistry::new();

        let tool = Tool {
            name: "get_time".to_string(),
            description: "Get current time".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        };

        registry.register_tool(tool);

        let definitions = registry.get_tool_definitions();
        assert_eq!(definitions.len(), 1);
        assert_eq!(definitions[0]["type"], "function");
        assert_eq!(definitions[0]["function"]["name"], "get_time");
    }

    #[test]
    fn test_default_registry() {
        let registry = create_default_registry();
        assert!(!registry.get_tools().is_empty());

        // Check that get_time tool is registered
        let time_tool = registry.find_tool("get_time");
        assert!(time_tool.is_some());
        assert_eq!(time_tool.unwrap().name, "get_time");
    }
}
