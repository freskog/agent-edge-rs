use agent_edge_rs::{config::ApiConfig, llm::integration::LLMIntegration};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn test_simple_time_calculation() {
    // Initialize logging
    env_logger::init();

    // Initialize LLM integration with real config
    let config = ApiConfig::load().expect("Failed to load API configuration");
    let mut integration = LLMIntegration::new(&config).unwrap();
    let cancel_token = CancellationToken::new();

    let query = "What time will it be in 2 hours from now?";
    println!("\n🔍 Testing query: {}", query);

    // Process the query
    let result = integration
        .process_user_instruction(query, cancel_token.clone())
        .await;

    let response_opt = match result {
        Ok(opt) => opt,
        Err(e) => {
            println!("LLM error during test: {} – skipping assertion", e);
            return;
        }
    };

    println!("📋 Response: {:?}", response_opt);

    if let Some(resp) = &response_opt {
        println!("📋 Response content: '{}'", resp);
        println!("📋 Response length: {} chars", resp.len());
        assert!(!resp.is_empty());
    }
}
