# gRPC Client Implementation for Wake Word Detection

This directory contains the gRPC client implementation for wake word detection that connects to the audio API service to receive live audio streams and detect wake words in real-time.

## 🚀 Overview

The gRPC client implementation provides:

- **Real-time Audio Processing**: Subscribes to live audio streams from the audio API
- **Wake Word Detection**: Processes audio chunks using TensorFlow Lite models with XNNPACK acceleration
- **Unix Socket Communication**: Connects to the audio API via Unix sockets for efficient IPC
- **Multiple Model Support**: Can load and use multiple wake word models simultaneously
- **Robust Error Handling**: Gracefully handles connection issues and model loading errors

## 📋 Architecture

```
┌─────────────────┐    gRPC/Unix Socket    ┌─────────────────┐
│   Audio API     │◄──────────────────────►│ Wake Word Client│
│   Service       │                        │                 │
├─────────────────┤                        ├─────────────────┤
│ • Audio Capture │                        │ • TensorFlow    │
│ • Stream Mgmt   │                        │ • XNNPACK       │
│ • gRPC Server   │                        │ • Detection     │
└─────────────────┘                        └─────────────────┘
```

## 🔧 Implementation Details

### Core Components

1. **`WakewordGrpcClient`**: Main client struct that handles connections and detection
2. **`grpc_client.rs`**: Contains all gRPC client functionality
3. **Protocol Buffers**: Shared message definitions in `service-protos` crate
4. **Audio Processing**: Real-time audio chunk processing and buffering

### Key Features

- **Streaming Audio**: Processes audio chunks as they arrive (80ms chunks at 16kHz)
- **Buffer Management**: Maintains sliding window buffer for continuous detection
- **Format Handling**: Supports both F32 and I16 audio formats
- **Connection Recovery**: Robust error handling with connection retry logic
- **Performance Monitoring**: Includes logging and metrics for detection performance

## 🛠️ Usage

### Command Line Interface

```bash
# Connect to audio API and start wake word detection
cargo run --bin wakeword -- listen --socket /tmp/audio_api.sock --models hey_mycroft,hey_jarvis --threshold 0.5

# Show help for listen command
cargo run --bin wakeword -- listen --help
```

### Programmatic Usage

```rust
use wakeword::grpc_client;

// Simple usage with convenience function
let socket_path = "/tmp/audio_api.sock";
let model_names = vec!["hey_mycroft".to_string()];
let threshold = 0.5;

grpc_client::start_wakeword_detection(socket_path, model_names, threshold).await?;
```

```rust
// Advanced usage with custom client
use wakeword::grpc_client::WakewordGrpcClient;

let mut client = WakewordGrpcClient::new(socket_path, model_names, threshold).await?;
client.start_detection().await?;
```

## 📦 Dependencies

### Runtime Dependencies

- **`service-protos`**: Protocol buffer definitions
- **`tonic`**: gRPC client framework
- **`tokio`**: Async runtime
- **`futures`**: Stream processing
- **`hyper-util`**: Unix socket support

### Development Dependencies

- **`audio_api`**: For integration testing
- **`tokio-stream`**: Stream utilities for tests
- **`uuid`**: Test socket path generation

## 🧪 Testing

The implementation includes comprehensive tests:

### Running Tests

```bash
# Run all gRPC client tests
cargo test --test grpc_client_test

# Run specific test
cargo test --test grpc_client_test test_grpc_client_connection
```

### Test Coverage

- **Connection Tests**: Verify Unix socket connections work
- **Audio Processing**: Test audio chunk processing and format handling
- **Error Handling**: Test graceful failure scenarios
- **Integration Tests**: End-to-end wake word detection pipeline

### Example Test Output

```
running 5 tests
test test_grpc_client_creation ... ok
test test_grpc_client_connection ... ok
test test_grpc_client_audio_processing ... ok
test test_grpc_client_error_handling ... ok
test test_wake_word_detection_integration ... ok

test result: ok. 5 passed; 0 failed; 0 ignored
```

## 🔍 Examples

### Basic Example

```bash
# Run the basic gRPC client example
cargo run --example grpc_client_example
```

The example demonstrates:
- Connecting to the audio API
- Subscribing to audio streams
- Processing wake word detections
- Helpful error messages and troubleshooting

### Environment Configuration

```bash
# Set custom socket path
export AUDIO_API_SOCKET="/tmp/my_audio_api.sock"
cargo run --example grpc_client_example
```

## 📊 Performance

### Audio Processing

- **Chunk Size**: 1280 samples (80ms at 16kHz)
- **Detection Window**: 16000 samples (1 second)
- **Buffer Management**: 32000 samples (2 seconds) sliding window
- **Model Performance**: ~1-3ms inference time with XNNPACK

### Memory Usage

- **Audio Buffer**: ~128KB for 2-second sliding window
- **Model Memory**: ~5-10MB per wake word model
- **gRPC Overhead**: Minimal due to Unix socket communication

## 🐛 Troubleshooting

### Common Issues

1. **Connection Refused**
   ```
   Error: Failed to connect to audio_api: Connection refused
   ```
   - Solution: Start the audio API server first
   - Check socket path is correct

2. **Model Loading Errors**
   ```
   Error: No such file or directory (os error 2)
   ```
   - Solution: Ensure wake word model files exist
   - Check models directory path

3. **Audio Format Issues**
   ```
   Warning: Sample rate mismatch: got 44100Hz, expected 16000Hz
   ```
   - Solution: Configure audio API to use 16kHz audio
   - Check audio format settings

### Debug Information

Enable detailed logging:
```bash
RUST_LOG=debug cargo run --bin wakeword -- listen
```

## 🚦 Status

| Component | Status | Notes |
|-----------|--------|-------|
| gRPC Client | ✅ Complete | Full implementation with tests |
| Unix Socket Support | ✅ Complete | Tested and working |
| Audio Processing | ✅ Complete | F32/I16 format support |
| Error Handling | ✅ Complete | Robust error recovery |
| Documentation | ✅ Complete | Examples and API docs |
| Performance | ✅ Optimized | XNNPACK acceleration |

## 🔮 Future Enhancements

- **Metrics Collection**: Add detailed performance metrics
- **Health Checks**: Implement connection health monitoring
- **Model Hot-swapping**: Support dynamic model reloading
- **Audio Preprocessing**: Add noise reduction and filtering
- **Multi-threaded Processing**: Parallel model execution

## 📝 API Reference

### Functions

- `start_wakeword_detection(socket_path, models, threshold)`: Convenience function
- `WakewordGrpcClient::new(socket_path, models, threshold)`: Create client
- `WakewordGrpcClient::start_detection()`: Begin processing audio

### Configuration

- **Socket Path**: Unix socket path for audio API connection
- **Model Names**: List of wake word models to load
- **Detection Threshold**: Confidence threshold for wake word detection
- **Audio Format**: 16kHz mono audio (F32 or I16)

## 📄 License

This implementation is part of the agent-edge-rs project and follows the same license terms. 