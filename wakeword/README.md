# OpenWakeWord Rust Implementation

This is a direct port of the Python OpenWakeWord implementation to Rust, providing wake word detection using TensorFlow Lite models.

## Overview

This implementation closely mirrors the Python OpenWakeWord structure and API for better compatibility and performance. The main differences from the previous implementation are:

### Key Changes

1. **Unified Model Class**: Similar to Python's `Model` class, handles all models (melspectrogram, embedding, wakeword) in a single interface
2. **AudioFeatures Class**: Equivalent to Python's `AudioFeatures` class for preprocessing and streaming audio
3. **Simplified API**: Direct port of the Python `predict()` method
4. **Better Buffer Management**: Matches Python's approach for streaming audio processing
5. **Model Loading**: Supports loading models by name or path, like the Python version
6. **TCP Protocol**: Uses simple TCP protocol instead of gRPC for audio streaming

### Architecture

The implementation follows the same three-stage architecture as the Python version:

1. **Melspectrogram Model**: Converts raw audio (1280 samples = 80ms) to mel features
2. **Embedding Model**: Converts mel features to 96-dimensional embeddings
3. **Wake Word Model**: Analyzes embedding sequences to detect wake words

## Usage

### Basic Example

```rust
use wakeword::Model;

// Create model instance (loads all pre-trained models by default)
let mut model = Model::default()?;

// Or load specific model
let mut model = Model::new(
    vec!["hey_mycroft".to_string()],
    vec![], // Use default class mappings
    0.0,    // No VAD filtering
    0.1,    // Default custom verifier threshold
)?;

// Process audio data (16-bit PCM, 16kHz, mono)
let audio_data: Vec<i16> = /* your audio data */;
let predictions = model.predict(&audio_data, None, 0.0)?;

// Check for detections
for (model_name, confidence) in predictions {
    if confidence > 0.5 {
        println!("Wake word '{}' detected with confidence: {:.6}", model_name, confidence);
    }
}
```

### Command Line Usage

```bash
# Test with audio file
cargo run -- test --input audio.wav --models hey_mycroft,hey_jarvis --threshold 0.5

# Live detection with TCP audio stream  
cargo run -- listen --server 127.0.0.1:50051 --models hey_mycroft --threshold 0.3

# Performance benchmark
cargo run -- benchmark --model hey_mycroft
```

### TCP Audio Streaming

The wakeword module connects to an audio server via TCP for real-time detection:

```rust
use wakeword::tcp_client;

// Simple synchronous API
let server_address = "127.0.0.1:50051";
let model_names = vec!["hey_mycroft".to_string()];
let threshold = 0.5;

tcp_client::start_wakeword_detection(server_address, model_names, threshold)?;
```

**Benefits of TCP over gRPC:**
- **Simpler**: No async complexity, straightforward blocking I/O
- **Faster**: Direct binary protocol, no protobuf overhead
- **Portable**: Works across networks, not just Unix sockets
- **Maintainable**: Easier to debug and understand for Scala developers

## Audio Format

The TCP protocol expects:
- **Sample Rate**: 16 kHz (required for wake word models)
- **Format**: 16-bit little-endian PCM
- **Channels**: Mono (single channel)
- **Chunk Size**: ~80ms chunks (1280 samples)

## Performance

### XNNPACK Acceleration

On ARM64 Linux systems, XNNPACK acceleration is automatically enabled for optimal performance:

```bash
# Check XNNPACK is working
cargo run -- benchmark --model hey_mycroft
```

Expected performance on ARM64:
- **Inference Time**: 10-30ms per 1.28s audio chunk
- **Real-time Factor**: 40-100x (much faster than real-time)
- **CPU Usage**: Low, suitable for battery-powered devices

### Synchronous Benefits

The synchronous design provides:
- **Low Latency**: No async scheduler overhead
- **Predictable Timing**: Direct function calls  
- **Simple Debugging**: Linear stack traces
- **Better for Real-time**: No task yielding or context switching

## Integration

### With Audio API

1. Start audio server:
```bash
cd audio_api && cargo run -- --address 127.0.0.1:50051
```

2. Connect wakeword client:
```bash
cd wakeword && cargo run -- listen --server 127.0.0.1:50051
```

### Programmatic Usage

```rust
use wakeword::tcp_client::WakewordClient;

// Create client
let mut client = WakewordClient::new(
    "127.0.0.1:50051",
    vec!["hey_mycroft".to_string()],
    0.5
)?;

// Start detection (blocks until stream ends)
client.start_detection()?;
```

## Dependencies

### Core Dependencies
- **`audio_protocol`**: TCP communication with audio server
- **`tflitec`**: TensorFlow Lite inference (with XNNPACK on ARM64)
- **`log`**: Logging framework
- **`clap`**: Command line interface

### Removed Dependencies
- ~~`tonic`~~: gRPC framework (replaced with TCP)
- ~~`futures`~~: Async streams (replaced with synchronous loops)
- ~~`tokio`~~: Async runtime (now synchronous)
- ~~`service-protos`~~: Protobuf definitions (replaced with binary protocol)

## Error Handling

Simple synchronous error handling:

```rust
match tcp_client::start_wakeword_detection(server, models, threshold) {
    Ok(()) => println!("Detection completed"),
    Err(e) => {
        eprintln!("Detection failed: {}", e);
        // Handle connection errors, model errors, etc.
    }
}
```

## Contributing

This implementation prioritizes:
1. **Simplicity**: Easy to understand and maintain
2. **Performance**: Low-latency real-time processing  
3. **Compatibility**: Matches Python OpenWakeWord behavior
4. **Reliability**: Robust error handling and recovery

Perfect for Scala developers who value clean, synchronous APIs over async complexity. 