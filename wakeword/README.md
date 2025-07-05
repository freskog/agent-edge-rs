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
# Test with default model
cargo run

# Test with specific model and threshold
cargo run -- --model hey_mycroft --threshold 0.3

# Test with audio file
cargo run -- --audio-file test.wav --threshold 0.5

# Enable debug logging
cargo run -- --debug
```

## Model Files

The implementation expects model files in the `models/` directory:

- `melspectrogram.tflite` - Mel spectrogram feature extraction
- `embedding_model.tflite` - Audio embedding model
- `hey_mycroft_v0.1.tflite` - Hey Mycroft wake word model
- Other wake word models...

## Performance

This implementation is designed to match the performance characteristics of the Python version:

- **Streaming Processing**: Processes audio in 80ms chunks
- **Memory Efficient**: Fixed-size buffers prevent memory growth
- **TensorFlow Lite**: Uses XNNPACK acceleration when available
- **Single-threaded**: Uses single-threaded inference for better control

## API Compatibility

The Rust API closely matches the Python version:

| Python | Rust |
|--------|------|
| `Model(wakeword_models=["hey_mycroft"])` | `Model::new(vec!["hey_mycroft".to_string()], ...)` |
| `model.predict(audio_data)` | `model.predict(&audio_data, None, 0.0)` |
| `model.reset()` | `model.reset()` |

## Dependencies

- `tflitec` - TensorFlow Lite C API bindings
- `hound` - WAV file I/O
- `clap` - Command line argument parsing
- `tokio` - Async runtime
- `log` - Logging
- `rand` - Random number generation

## Building

```bash
cargo build --release
```

## Testing

```bash
# Run tests
cargo test

# Run with example audio
cargo run -- --audio-file example.wav
``` 