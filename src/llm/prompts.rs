// System prompts and conversation templates

pub struct SystemPrompts;

impl SystemPrompts {
    /// Default assistant system prompt
    pub fn default_assistant() -> &'static str {
        "You are a helpful, harmless, and honest AI assistant. \
         You provide clear, accurate, and concise responses. \
         If you're unsure about something, you say so rather than guessing. \
         You aim to be respectful and professional in all interactions."
    }

    /// Voice assistant specific prompt
    pub fn voice_assistant() -> &'static str {
        "You are a voice AI assistant designed for natural spoken conversation. \
         Keep your responses conversational, concise, and appropriate for speech. \
         Avoid using formatting like markdown, bullet points, or long lists unless specifically requested. \
         Speak naturally as if having a real conversation. \
         If you need to present information, do so in a flowing, spoken format rather than structured text. \
         Keep responses under 100 words unless more detail is specifically requested."
    }

    /// Smart home assistant prompt
    pub fn smart_home_assistant() -> &'static str {
        "You are a smart home AI assistant. You can help with:
         - Controlling lights, temperature, and other devices
         - Setting reminders and timers
         - Answering questions about home automation
         - Providing weather updates and news
         - Playing music and entertainment
         
         Keep responses brief and action-oriented.
         When controlling devices, confirm actions taken.
         Always prioritize user safety and privacy."
    }

    /// Technical assistant prompt
    pub fn technical_assistant() -> &'static str {
        "You are a technical AI assistant with expertise in:
         - Programming and software development
         - System administration and DevOps
         - Hardware and electronics
         - Troubleshooting and problem-solving
         
         Provide accurate technical information, code examples when helpful,
         and step-by-step solutions for technical problems.
         Ask clarifying questions when the technical context is unclear."
    }

    /// Creative assistant prompt
    pub fn creative_assistant() -> &'static str {
        "You are a creative AI assistant that helps with:
         - Writing and storytelling
         - Brainstorming and ideation
         - Art and design concepts
         - Music and creative projects
         
         Be imaginative, inspiring, and supportive of creative endeavors.
         Offer multiple perspectives and alternatives when appropriate.
         Encourage experimentation and creative thinking."
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
        assert!(!SystemPrompts::default_assistant().is_empty());
        assert!(!SystemPrompts::voice_assistant().is_empty());
        assert!(!SystemPrompts::smart_home_assistant().is_empty());
        assert!(!SystemPrompts::technical_assistant().is_empty());
        assert!(!SystemPrompts::creative_assistant().is_empty());

        // Voice assistant should mention conversation style
        assert!(SystemPrompts::voice_assistant().contains("conversational"));
    }

    #[test]
    fn test_conversation_templates() {
        let greeting = ConversationTemplates::wake_word_greeting("Jarvis");
        assert!(greeting.contains("Jarvis"));
        assert!(greeting.contains("help"));

        let error = ConversationTemplates::voice_error("audio");
        assert!(error.contains("trouble hearing"));

        let unknown_error = ConversationTemplates::voice_error("unknown");
        assert!(error.contains("wrong"));
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

    #[test]
    fn test_tool_messages() {
        let tool_msg = ConversationTemplates::tool_execution("calculator");
        assert!(tool_msg.contains("calculator"));

        let completion_msg = ConversationTemplates::task_completed("your calculation");
        assert!(completion_msg.contains("calculation"));
        assert!(completion_msg.contains("completed"));
    }
}
