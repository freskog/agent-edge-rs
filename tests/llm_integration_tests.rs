use agent_edge_rs::{
    config::ApiConfig,
    llm::{tools::create_default_registry, LLMIntegration},
};
use std::env;
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn test_time_tool_direct() {
    // Skip if no API key
    if env::var("GROQ_API_KEY").is_err() {
        println!("GROQ_API_KEY not set, skipping integration test");
        return;
    }

    let config = ApiConfig::load().expect("Failed to load config");
    let mut integration = LLMIntegration::new(&config).expect("Failed to create integration");
    let cancel = CancellationToken::new();

    // Test direct time query
    let response = integration
        .process_user_instruction("What time is it?", cancel)
        .await;
    assert!(
        response.is_ok(),
        "Failed to process time query: {:?}",
        response.err()
    );

    let response_text = response.unwrap();
    println!("Time query response: {:?}", response_text);

    // Should contain time-related content
    assert!(response_text.is_some());
    let response_text = response_text.unwrap();
    assert!(
        response_text.to_lowercase().contains("time")
            || response_text.to_lowercase().contains("it's")
            || response_text.contains(":")
    );
}

#[tokio::test]
async fn test_time_tool_variations() {
    // Skip if no API key
    if env::var("GROQ_API_KEY").is_err() {
        println!("GROQ_API_KEY not set, skipping integration test");
        return;
    }

    let config = ApiConfig::load().expect("Failed to load config");
    let mut integration = LLMIntegration::new(&config).expect("Failed to create integration");

    let test_queries = vec![
        "What time is it?",
        "Tell me the time",
        "Time",
        "What's the current time?",
    ];

    for query in test_queries {
        let cancel = CancellationToken::new();
        let response = integration.process_user_instruction(query, cancel).await;
        assert!(
            response.is_ok(),
            "Failed to process query '{}': {:?}",
            query,
            response.err()
        );

        let response_text = response.unwrap();
        println!("Query: '{}' -> Response: {:?}", query, response_text);

        // Should get a reasonable response
        assert!(response_text.is_some());
        let response_text = response_text.unwrap();
        assert!(response_text.len() > 5);
    }
}

#[tokio::test]
async fn test_tool_registry() {
    let registry = create_default_registry();
    assert!(!registry.get_tools().is_empty());

    // Check that get_time tool is available
    let time_tool = registry.find_tool("get_time");
    assert!(time_tool.is_some());
    assert_eq!(time_tool.unwrap().name, "get_time");
}

#[tokio::test]
async fn test_tool_definitions() {
    let registry = agent_edge_rs::llm::tools::create_default_registry();
    let definitions = registry.get_tool_definitions();

    assert!(!definitions.is_empty());

    // Check that get_time tool definition is correct
    let time_definition = definitions
        .iter()
        .find(|def| def["function"]["name"] == "get_time");
    assert!(time_definition.is_some());

    let time_def = time_definition.unwrap();
    assert_eq!(time_def["type"], "function");
    assert_eq!(time_def["function"]["name"], "get_time");
    assert!(time_def["function"]["description"]
        .as_str()
        .unwrap()
        .contains("time"));
}

#[tokio::test]
async fn test_conversation_context() {
    // Skip if no API key
    if env::var("GROQ_API_KEY").is_err() {
        println!("GROQ_API_KEY not set, skipping integration test");
        return;
    }

    let config = ApiConfig::load().expect("Failed to load config");
    let mut integration = LLMIntegration::new(&config).expect("Failed to create integration");

    // First query
    let response1 = integration
        .process_user_instruction("What time is it?", CancellationToken::new())
        .await
        .unwrap();
    assert!(response1.is_some());
    let response1_text = response1.unwrap();
    println!("First response: {:?}", response1_text);

    // Second query - should maintain context
    let response2 = integration
        .process_user_instruction("What about now?", CancellationToken::new())
        .await
        .unwrap();
    assert!(response2.is_some());
    let response2_text = response2.unwrap();
    println!("Second response: {:?}", response2_text);

    // Both should be valid responses
    assert!(!response1_text.is_empty());
    assert!(!response2_text.is_empty());

    // Context should have messages
    let summary = integration.context_summary();
    println!("Context summary: {:?}", summary);
    assert!(summary.contains("messages"));
}

#[tokio::test]
async fn test_llm_integration() {
    // Skip if no API key
    if env::var("GROQ_API_KEY").is_err() {
        println!("GROQ_API_KEY not set, skipping integration test");
        return;
    }

    let config = ApiConfig::load().expect("Failed to load config");
    let mut integration = LLMIntegration::new(&config).expect("Failed to create integration");
    let cancel = CancellationToken::new();

    let response = integration
        .process_user_instruction("What time is it?", cancel)
        .await;
    assert!(response.is_ok());
    let response = response.unwrap();
    assert!(response.is_some());
}

#[tokio::test]
async fn test_llm_integration_with_context() {
    // Skip if no API key
    if env::var("GROQ_API_KEY").is_err() {
        println!("GROQ_API_KEY not set, skipping integration test");
        return;
    }

    let config = ApiConfig::load().expect("Failed to load config");
    let mut integration = LLMIntegration::new(&config).expect("Failed to create integration");

    let queries = vec![
        "What time is it?",
        "What about now?",
        "And now?",
        "One more time?",
    ];

    for query in queries {
        let cancel = CancellationToken::new();
        let response = integration.process_user_instruction(query, cancel).await;
        assert!(response.is_ok());
        let response = response.unwrap();
        assert!(response.is_some());
    }
}

#[tokio::test]
async fn test_llm_integration_with_memory() {
    // Skip if no API key
    if env::var("GROQ_API_KEY").is_err() {
        println!("GROQ_API_KEY not set, skipping integration test");
        return;
    }

    let config = ApiConfig::load().expect("Failed to load config");
    let mut integration = LLMIntegration::new(&config).expect("Failed to create integration");

    // First query
    let cancel1 = CancellationToken::new();
    let response1 = integration
        .process_user_instruction("What time is it?", cancel1)
        .await;
    assert!(response1.is_ok());
    let response1 = response1.unwrap();
    assert!(response1.is_some());
    let response1_text = response1.unwrap();
    println!("First response: {:?}", response1_text);
    assert!(!response1_text.is_empty());

    // The LLM should remember the previous question and understand the context
    let cancel2 = CancellationToken::new();
    let response2 = integration
        .process_user_instruction("What about now?", cancel2)
        .await;
    assert!(response2.is_ok());
    let response2 = response2.unwrap();
    assert!(response2.is_some());
    let response2_text = response2.unwrap();
    println!("Second response: {:?}", response2_text);
    assert!(!response2_text.is_empty());
}
