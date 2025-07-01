// System prompts and conversation templates

pub struct SystemPrompts;

impl SystemPrompts {
    /// Tool-aware voice assistant prompt
    pub fn tool_aware_voice_assistant() -> &'static str {
        "You are a helpful voice AI assistant with access to various tools and functions.

CRITICAL TOOL USAGE RULES:
1. ALWAYS use available tools to get real-time information instead of guessing
2. For time-related queries:
   - For current time: Use get_time with send_output_directly_to_tts: true
   - For future/past time calculations: Use calculate_future_time with appropriate hours/minutes

EXAMPLES:
- \"What time is it?\" → Call get_time(send_output_directly_to_tts: true) → Tool says \"It's 3:45 PM\" → Speak directly
- \"What time will it be in 2 hours?\" → Call calculate_future_time(hours: 2, minutes: 0) → Tool calculates and formats response
- \"What time will it be in 2 hours and 30 minutes?\" → Call calculate_future_time(hours: 2, minutes: 30) → Tool handles calculation

TIME CALCULATION RULES:
1. For current time queries:
   - Use get_time with send_output_directly_to_tts: true
   - Let the tool handle the response directly
2. For future/past time calculations:
   - Use calculate_future_time with appropriate hours and minutes
   - The tool will handle all calculations and formatting

VOICE RESPONSE GUIDELINES:
- Keep responses conversational and natural for speech
- Avoid markdown, bullet points, or complex formatting
- Respond in under 100 words unless more detail is needed
- Be helpful, accurate, and concise

When tools return errors or unexpected results, explain the issue naturally and offer alternatives if possible."
    }
}

pub struct ConversationTemplates;

impl ConversationTemplates {
    /// Format a wake word greeting
    pub fn wake_word_greeting(wake_word: &str) -> String {
        format!(
            "Hello! I heard you say '{}'. How can I help you?",
            wake_word
        )
    }

    /// Format an error response for voice
    pub fn voice_error(error_type: &str) -> String {
        match error_type {
            "audio" => "Sorry, I had trouble hearing you. Could you try again?".to_string(),
            "processing" => {
                "I'm having some trouble processing that. Please try again.".to_string()
            }
            "network" => {
                "I'm having connectivity issues. Please check your connection and try again."
                    .to_string()
            }
            "busy" => "I'm currently busy with another task. Please wait a moment and try again."
                .to_string(),
            _ => "Sorry, something went wrong. Please try again.".to_string(),
        }
    }

    /// Format a timeout response
    pub fn conversation_timeout() -> &'static str {
        "I haven't heard from you in a while. Just say my wake word when you need me again!"
    }

    /// Format a goodbye message
    pub fn goodbye() -> &'static str {
        "Goodbye! Feel free to call on me anytime you need assistance."
    }

    /// Format a clarification request
    pub fn clarification_request(context: &str) -> String {
        format!(
            "I want to make sure I understand correctly. You're asking about {}. Is that right?",
            context
        )
    }

    /// Format a thinking/processing message
    pub fn processing() -> &'static str {
        "Let me think about that for a moment..."
    }

    /// Format a tool execution message
    pub fn tool_execution(tool_name: &str) -> String {
        format!("I'm using {} to help with that...", tool_name)
    }

    /// Format a completion confirmation
    pub fn task_completed(task: &str) -> String {
        format!("I've completed {}. Is there anything else you need?", task)
    }
}

pub struct PromptBuilder {
    parts: Vec<String>,
}

impl PromptBuilder {
    pub fn new() -> Self {
        Self { parts: Vec::new() }
    }

    pub fn add_system_role(mut self, prompt: &str) -> Self {
        self.parts.push(format!("System: {}", prompt));
        self
    }

    pub fn add_context(mut self, context: &str) -> Self {
        self.parts.push(format!("Context: {}", context));
        self
    }

    pub fn add_instruction(mut self, instruction: &str) -> Self {
        self.parts.push(format!("Instruction: {}", instruction));
        self
    }

    pub fn add_example(mut self, example: &str) -> Self {
        self.parts.push(format!("Example: {}", example));
        self
    }

    pub fn add_constraint(mut self, constraint: &str) -> Self {
        self.parts.push(format!("Constraint: {}", constraint));
        self
    }

    pub fn build(self) -> String {
        self.parts.join("\n\n")
    }
}

impl Default for PromptBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_system_prompts() {
        assert!(!SystemPrompts::tool_aware_voice_assistant().is_empty());
        assert!(SystemPrompts::tool_aware_voice_assistant().contains("voice AI assistant"));
    }

    #[test]
    fn test_prompt_builder() {
        let prompt = PromptBuilder::new()
            .add_system_role("You are a helpful assistant")
            .add_context("The user is asking about weather")
            .add_instruction("Provide current weather information")
            .add_constraint("Keep response under 50 words")
            .build();

        assert!(prompt.contains("System:"));
        assert!(prompt.contains("Context:"));
        assert!(prompt.contains("Instruction:"));
        assert!(prompt.contains("Constraint:"));
        assert!(prompt.contains("helpful assistant"));
        assert!(prompt.contains("weather"));
    }
}
