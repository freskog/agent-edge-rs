# Mock Audio Server for Testing

The `audio` crate now includes a `MockAudioServer` that can be started programmatically from tests, eliminating the need to run a separate process or have live microphone input.

## ✅ Benefits for Testing

- **Deterministic**: Uses controlled audio files instead of live input
- **Fast**: Configurable playback speed for rapid testing
- **Isolated**: Each test gets its own server instance on a random port
- **Automatic**: Server starts/stops automatically with test lifecycle
- **Flexible**: Works with any 16kHz mono s16le WAV file

## 🚀 Quick Example

```rust
use audio::{MockAudioServer, MockServerConfig};
use std::path::PathBuf;

#[test]
fn test_with_mock_audio() {
    // Configure the mock server
    let config = MockServerConfig {
        audio_file: PathBuf::from("../tests/data/hey_mycroft_test.wav"),
        bind_address: "127.0.0.1:0".to_string(), // Random port
        loop_audio: true,
        speed: 4.0, // 4x speed for faster testing
        ..Default::default()
    };

    // Start server in background - gets random port automatically
    let mock_server = MockAudioServer::new(config)
        .expect("Failed to create mock server")
        .start_background()
        .expect("Failed to start mock server");

    println!("Mock server running on {}", mock_server.address());

    // Now your test can:
    // 1. Point wakeword service to mock_server.address() 
    // 2. Start agent with wakeword service
    // 3. Verify expected behavior

    // Server automatically stops when mock_server is dropped
}
```

See `audio/tests/mock_server_test.rs` for complete working examples.

## 🎯 Integration Test Pattern

```rust
#[test]
fn test_end_to_end_wake_word_detection() {
    // 1. Start mock audio server with wake word file
    let mock_server = start_mock_server("../tests/data/hey_mycroft_test.wav");
    
    // 2. Start wakeword service pointing to mock server
    let wakeword_server = start_wakeword_service(&mock_server.address());
    
    // 3. Start agent pointing to wakeword server  
    let agent = start_agent(&wakeword_server.address());
    
    // 4. Verify wake word detection and agent response
    let response = agent.wait_for_response(Duration::from_secs(5));
    assert!(response.contains("Hello, how can I help?"));
    
    // All servers stop automatically when dropped
}
```

## 📁 Available Test Files

Located in `tests/data/`:

- `hey_mycroft_test.wav` - Clean wake word recording
- `alexa_test.wav` - Alternative wake word (for false positive testing)
- `immediate_what_time_is_it.wav` - Wake word + immediate question
- `delay_start_what_time_is_it.wav` - Wake word + delayed question
- `hesitation_what_time_is_it.wav` - Wake word + hesitant speech

## ⚡ Configuration Options

```rust
pub struct MockServerConfig {
    pub audio_file: PathBuf,        // WAV file to play
    pub bind_address: String,       // Use "127.0.0.1:0" for random port
    pub loop_audio: bool,           // Repeat file continuously
    pub silence_duration: f32,      // Seconds between loops
    pub speed: f32,                 // Playback speed (1.0 = real time)
}
```

## 🧪 Test Scenarios

### Basic Wake Word Detection
```rust
let config = MockServerConfig {
    audio_file: PathBuf::from("../tests/data/hey_mycroft_test.wav"),
    speed: 8.0, // Very fast for quick test
    ..Default::default()
};
```

### VAD Boundary Testing
```rust
let config = MockServerConfig {
    audio_file: PathBuf::from("../tests/data/hesitation_what_time_is_it.wav"),
    loop_audio: true,
    speed: 2.0,
    ..Default::default()
};
```

### Multi-Turn Conversation Testing
```rust
let config = MockServerConfig {
    audio_file: PathBuf::from("../tests/data/immediate_what_time_is_it.wav"),
    loop_audio: true,
    silence_duration: 8.0, // Long pause for follow-up timeout testing
    ..Default::default()
};
```

### Performance/Stress Testing
```rust
let config = MockServerConfig {
    audio_file: PathBuf::from("../tests/data/hey_mycroft_test.wav"),
    loop_audio: true,
    speed: 10.0, // Very fast
    silence_duration: 0.1, // Minimal pause
    ..Default::default()
};
```

## 🔧 Running Tests

```bash
# Run the example tests
cd audio
cargo test mock_server

# Run specific test
cargo test test_mock_server_basic_functionality

# Run with logging
RUST_LOG=info cargo test mock_server -- --nocapture
```

## 🎯 Integration with Existing Tests

You can add the mock server to existing integration tests:

```rust
// In tests/integration_tests.rs or similar
use audio::{MockAudioServer, MockServerConfig};

fn setup_mock_audio() -> audio::MockServerHandle {
    let config = MockServerConfig {
        audio_file: PathBuf::from("tests/data/hey_mycroft_test.wav"),
        speed: 4.0, // Faster testing
        ..Default::default()
    };
    
    MockAudioServer::new(config)
        .unwrap()
        .start_background()
        .unwrap()
}

#[test]
fn test_full_pipeline() {
    let mock_audio = setup_mock_audio();
    
    // Point your services to mock_audio.address()
    // instead of real audio server
    
    // Test your pipeline...
}
```

This approach gives you **fast, reliable, repeatable tests** without dependency on hardware or external processes! 🎉 