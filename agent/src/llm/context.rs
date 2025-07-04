use super::client::Message;
use std::collections::VecDeque;

#[derive(Debug, Clone)]
pub struct ConversationContext {
    messages: VecDeque<Message>,
    max_messages: usize,
    max_tokens: usize,
    system_message: Option<Message>,
}

impl ConversationContext {
    /// Create a new conversation context
    pub fn new(max_messages: usize, max_tokens: usize) -> Self {
        Self {
            messages: VecDeque::new(),
            max_messages,
            max_tokens,
            system_message: None,
        }
    }

    /// Create with default limits (20 messages, ~8000 tokens)
    pub fn with_defaults() -> Self {
        Self::new(20, 8000)
    }

    /// Set the system message
    pub fn set_system_message(&mut self, content: impl Into<String>) {
        self.system_message = Some(Message::system(content));
    }

    /// Add a user message
    pub fn add_user_message(&mut self, content: impl Into<String>) {
        self.add_message(Message::user(content));
    }

    /// Add an assistant message
    pub fn add_assistant_message(&mut self, content: impl Into<String>) {
        self.add_message(Message::assistant(content));
    }

    /// Add a message to the context
    pub fn add_message(&mut self, message: Message) {
        self.messages.push_back(message);
        self.trim_context();
    }

    /// Get all messages for API call (includes system message if set)
    pub fn get_messages(&self) -> Vec<Message> {
        let mut messages = Vec::new();

        // Add system message first if present
        if let Some(ref system_msg) = self.system_message {
            messages.push(system_msg.clone());
        }

        // Add conversation messages
        messages.extend(self.messages.iter().cloned());

        messages
    }

    /// Get the last N messages
    pub fn get_last_messages(&self, n: usize) -> Vec<Message> {
        let mut messages = Vec::new();

        // Always include system message if present
        if let Some(ref system_msg) = self.system_message {
            messages.push(system_msg.clone());
        }

        // Add last N conversation messages
        let start_idx = if self.messages.len() > n {
            self.messages.len() - n
        } else {
            0
        };

        for i in start_idx..self.messages.len() {
            if let Some(msg) = self.messages.get(i) {
                messages.push(msg.clone());
            }
        }

        messages
    }

    /// Clear all messages except system message
    pub fn clear(&mut self) {
        self.messages.clear();
    }

    /// Get conversation length (excluding system message)
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    /// Check if conversation is empty (excluding system message)
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// Estimate token count (rough approximation)
    fn estimate_tokens(&self) -> usize {
        let mut total = 0;

        // System message tokens
        if let Some(ref system_msg) = self.system_message {
            total += self.estimate_message_tokens(system_msg);
        }

        // Conversation message tokens
        for message in &self.messages {
            total += self.estimate_message_tokens(message);
        }

        total
    }

    /// Rough token estimation for a single message
    fn estimate_message_tokens(&self, message: &Message) -> usize {
        // Rough approximation: 1 token ≈ 4 characters for English text
        // Add some overhead for role and formatting
        (message.content.len() / 4) + (message.role.len() / 4) + 10
    }

    /// Trim context to stay within limits
    fn trim_context(&mut self) {
        // Trim by message count
        while self.messages.len() > self.max_messages {
            self.messages.pop_front();
        }

        // Trim by token count (rough estimation)
        while self.estimate_tokens() > self.max_tokens && !self.messages.is_empty() {
            self.messages.pop_front();
        }
    }

    /// Get context summary for debugging
    pub fn summary(&self) -> String {
        format!(
            "Context: {} messages, ~{} tokens (limits: {} messages, {} tokens)",
            self.len(),
            self.estimate_tokens(),
            self.max_messages,
            self.max_tokens
        )
    }

    /// Export conversation to JSON for persistence
    pub fn to_json(&self) -> serde_json::Result<String> {
        use serde_json::json;

        let messages_json: Vec<serde_json::Value> = self
            .messages
            .iter()
            .map(|msg| {
                json!({
                    "role": msg.role,
                    "content": msg.content
                })
            })
            .collect();

        let context_json = json!({
            "system_message": self.system_message.as_ref().map(|msg| json!({
                "role": msg.role,
                "content": msg.content
            })),
            "messages": messages_json,
            "max_messages": self.max_messages,
            "max_tokens": self.max_tokens
        });

        serde_json::to_string_pretty(&context_json)
    }

    /// Import conversation from JSON
    pub fn from_json(json_str: &str) -> serde_json::Result<Self> {
        let json: serde_json::Value = serde_json::from_str(json_str)?;

        let max_messages = json["max_messages"].as_u64().unwrap_or(20) as usize;
        let max_tokens = json["max_tokens"].as_u64().unwrap_or(8000) as usize;

        let mut context = Self::new(max_messages, max_tokens);

        // Load system message if present
        if let Some(system_json) = json["system_message"].as_object() {
            if let (Some(role), Some(content)) = (
                system_json["role"].as_str(),
                system_json["content"].as_str(),
            ) {
                if role == "system" {
                    context.set_system_message(content);
                }
            }
        }

        // Load messages
        if let Some(messages_array) = json["messages"].as_array() {
            for msg_json in messages_array {
                if let (Some(role), Some(content)) =
                    (msg_json["role"].as_str(), msg_json["content"].as_str())
                {
                    let message = Message {
                        role: role.to_string(),
                        content: content.to_string(),
                    };
                    context.add_message(message);
                }
            }
        }

        Ok(context)
    }
}

impl Default for ConversationContext {
    fn default() -> Self {
        Self::with_defaults()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_creation() {
        let context = ConversationContext::new(5, 1000);
        assert_eq!(context.len(), 0);
        assert!(context.is_empty());
        assert_eq!(context.max_messages, 5);
        assert_eq!(context.max_tokens, 1000);
    }

    #[test]
    fn test_message_addition() {
        let mut context = ConversationContext::new(5, 1000);

        context.set_system_message("You are a helpful assistant");
        context.add_user_message("Hello");
        context.add_assistant_message("Hi there!");

        assert_eq!(context.len(), 2); // System message not counted in len()

        let messages = context.get_messages();
        assert_eq!(messages.len(), 3); // System + 2 conversation messages
        assert_eq!(messages[0].role, "system");
        assert_eq!(messages[1].role, "user");
        assert_eq!(messages[2].role, "assistant");
    }

    #[test]
    fn test_message_trimming() {
        let mut context = ConversationContext::new(3, 10000);

        // Add more messages than the limit
        for i in 0..5 {
            context.add_user_message(format!("Message {}", i));
        }

        assert_eq!(context.len(), 3); // Should be trimmed to max_messages

        let messages = context.get_messages();
        let last_message = messages.last().unwrap();
        assert!(last_message.content.contains("Message 4")); // Last message should be preserved
    }

    #[test]
    fn test_get_last_messages() {
        let mut context = ConversationContext::new(10, 10000);
        context.set_system_message("System");

        for i in 0..5 {
            context.add_user_message(format!("User {}", i));
            context.add_assistant_message(format!("Assistant {}", i));
        }

        let last_3 = context.get_last_messages(3);
        assert_eq!(last_3.len(), 4); // System + 3 conversation messages
        assert_eq!(last_3[0].role, "system");

        // Should get the last 3 conversation messages
        assert!(last_3[last_3.len() - 1].content.contains("Assistant 4"));
    }

    #[test]
    fn test_clear() {
        let mut context = ConversationContext::new(10, 10000);
        context.set_system_message("System");
        context.add_user_message("Hello");
        context.add_assistant_message("Hi");

        assert_eq!(context.len(), 2);

        context.clear();
        assert_eq!(context.len(), 0);

        // System message should be preserved
        let messages = context.get_messages();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "system");
    }

    #[test]
    fn test_token_estimation() {
        let context = ConversationContext::new(10, 10000);
        let message = Message::user("This is a test message with some content");

        let estimated = context.estimate_message_tokens(&message);
        assert!(estimated > 0);
        assert!(estimated < 50); // Should be reasonable for this short message
    }

    #[test]
    fn test_json_serialization() {
        let mut context = ConversationContext::new(5, 1000);
        context.set_system_message("You are helpful");
        context.add_user_message("Hello");
        context.add_assistant_message("Hi there!");

        let json = context.to_json().unwrap();
        let restored = ConversationContext::from_json(&json).unwrap();

        assert_eq!(restored.len(), context.len());
        assert_eq!(restored.max_messages, context.max_messages);
        assert_eq!(restored.max_tokens, context.max_tokens);

        let original_messages = context.get_messages();
        let restored_messages = restored.get_messages();
        assert_eq!(original_messages.len(), restored_messages.len());
    }
}
