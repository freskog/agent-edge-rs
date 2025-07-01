use agent_edge_rs::{config::ApiConfig, llm::integration::LLMIntegration};
use regex::Regex;
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn test_time_calculations() {
    // Initialize LLM integration with real config
    let config = ApiConfig::load().expect("Failed to load API configuration");
    let mut integration = LLMIntegration::new(&config).unwrap();
    let cancel_token = CancellationToken::new();

    // Test cases for time calculations
    let test_cases = vec![
        "What time is it?",
        "What time will it be in 2 hours from now?",
        "What time will it be in 2 hours and 30 minutes?",
        "What time will it be in 30 minutes?",
    ];

    // Compile regex pattern once
    let time_pattern = Regex::new(r"(?i)\d{1,2}:\d{2}\s*[AP]M").unwrap();

    for query in test_cases {
        println!("\nTesting query: {}", query);

        // Process the query
        let response = integration
            .process_user_instruction(query, cancel_token.clone())
            .await
            .unwrap();

        // Verify we got a response
        assert!(response.is_some(), "Should get a response for: {}", query);

        let response = response.unwrap();
        println!("Response: {}", response);

        // Verify time format
        let matches: Vec<_> = time_pattern
            .find_iter(&response)
            .map(|m| m.as_str())
            .collect();
        println!("Time pattern matches: {:?}", matches);
        assert!(
            !matches.is_empty(),
            "Response should contain time in H:MM AM/PM or HH:MM AM/PM format: {}",
            response
        );

        // Verify the response makes sense for the query
        match query {
            "What time is it?" => {
                assert!(
                    response.starts_with("It's"),
                    "Current time response should start with 'It's'"
                );
            }
            _ => {
                assert!(
                    response.contains("will be"),
                    "Future time response should contain 'will be'"
                );
                if query.contains("hours") && query.contains("minutes") {
                    assert!(
                        response.contains("hours") && response.contains("minutes"),
                        "Response should mention both hours and minutes: {}",
                        response
                    );
                } else if query.contains("hours") {
                    assert!(
                        response.contains("hours"),
                        "Response should mention hours: {}",
                        response
                    );
                } else if query.contains("minutes") {
                    assert!(
                        response.contains("minutes"),
                        "Response should mention minutes: {}",
                        response
                    );
                }
            }
        }
    }
}
