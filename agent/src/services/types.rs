use serde_json::Value;

/// Tool call from LLM
#[derive(Debug, Clone)]
pub struct ToolCall {
    pub name: String,
    pub text: String,
}

/// LLM response containing tool calls
#[derive(Debug, Clone)]
pub struct LLMResponse {
    pub tool_calls: Vec<ToolCall>,
}

/// Simple tool registry for blocking LLM
pub struct ToolRegistry {
    tools: Vec<Value>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self { tools: Vec::new() }
    }

    pub fn get_tool_definitions(&self) -> &[Value] {
        &self.tools
    }
}

/// Create default tool registry with respond tool
pub fn create_default_registry() -> ToolRegistry {
    let mut registry = ToolRegistry::new();

    // Add respond tool definition
    let respond_tool = serde_json::json!({
        "type": "function",
        "function": {
            "name": "respond",
            "description": "Respond to the user with text that will be spoken aloud",
            "parameters": {
                "type": "object",
                "properties": {
                    "message": {
                        "type": "string",
                        "description": "The message to speak to the user"
                    }
                },
                "required": ["message"]
            }
        }
    });

    registry.tools.push(respond_tool);
    registry
}

/// Audio chunk for buffering
#[derive(Debug, Clone)]
pub struct AudioChunk {
    pub samples: Vec<f32>,
    pub timestamp: std::time::Instant,
}

impl From<audio_protocol::AudioChunk> for AudioChunk {
    fn from(chunk: audio_protocol::AudioChunk) -> Self {
        // Convert raw bytes (s16le format) to f32 samples
        let samples: Vec<f32> = chunk
            .data
            .chunks_exact(2)
            .map(|bytes| {
                let sample_i16 = i16::from_le_bytes([bytes[0], bytes[1]]);
                sample_i16 as f32 / 32768.0 // Convert to [-1.0, 1.0] range
            })
            .collect();

        Self {
            samples,
            timestamp: std::time::Instant::now(),
        }
    }
}
