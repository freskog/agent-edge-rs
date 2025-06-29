# Agent Edge RS - OpenWakeWord Detection System

A high-performance Rust implementation of OpenWakeWord for real-time "Hey Mycroft" detection on edge devices.

## 🎯 Overview

This project provides a complete wake word detection solution optimized for edge devices:

- **🎤 Real-time Audio**: PulseAudio integration with 6-channel ReSpeaker support
- **🧠 OpenWakeWord Pipeline**: 3-stage ML pipeline with TensorFlow Lite models
- **⚡ VAD Optimization**: WebRTC Voice Activity Detection reduces CPU usage by 80-90%
- **🔄 Debouncing**: Prevents repeated detections from single utterances
- **🎯 High Accuracy**: Peak confidence 1.0 on test data, minimal false positives

## 🚀 Quick Start

### DevContainer (Recommended)

The easiest way to develop and test:

1. **Prerequisites**: [VS Code](https://code.visualstudio.com/) + [Dev Containers extension](https://marketplace.visualstudio.com/items?itemName=ms-vscode-remote.remote-containers)

2. **Clone and open**:
   ```bash
   git clone <repository-url>
   code agent-edge-rs
   ```

3. **Reopen in container**: VS Code will prompt - click "Reopen in Container"

4. **Build and test**:
   ```bash
   # Build the project for ARM64 (Pi deployment)
   cargo build --release
   
   # Run comprehensive tests
   cargo test
   
   # First-time deployment to your Pi
   ./deploy-to-pi.sh --full myuser@192.168.1.100
   
   # Quick updates during development
   ./deploy-to-pi.sh myuser@192.168.1.100
   ```

### Supported Platform

**Raspberry Pi Only**: This project is specifically designed for Raspberry Pi deployment (Pi Zero 2W minimum, Pi 3+ recommended). Development should be done using the DevContainer which provides the correct ARM64 build environment.

**Note**: Native Linux builds are not currently supported. Use the DevContainer for development and the deployment script for Pi deployment.

## 🎤 Usage

After deployment, run on your Raspberry Pi:

```bash
# SSH to your Pi and run the agent
ssh myuser@192.168.1.100
cd agent-edge
./run-agent.sh

# Or with custom logging
RUST_LOG=info ./run-agent.sh
```

The system will:
1. **Initialize** the 3-stage detection pipeline
2. **Start WebRTC VAD** for CPU optimization
3. **Begin listening** for "Hey Mycroft"
4. **Display detection** with confidence scores

### Expected Output

```
🎤 Microphone initialized
🔊 VAD initialized (Aggressive mode, 16kHz)
🤖 Detection pipeline ready

🚨🎉 WAKEWORD DETECTED! 🎉🚨
   Confidence: 1.000
   Say 'Hey Mycroft' to trigger again!
```

## 🏗️ Architecture

### Pipeline Flow

```text
Audio Input → VAD Filter → 3-Stage ML Pipeline → Debounced Detection
(6-channel)   (CPU opt)   (80ms chunks)         (1-second cooldown)
```

### 3-Stage OpenWakeWord Pipeline

1. **Melspectrogram** (`melspectrogram.tflite`)
   - Input: 1280 audio samples (80ms at 16kHz)
   - Output: 160 mel features (5×32 frames)
   - Purpose: Audio "tokenization" into acoustic features

2. **Embedding** (`embedding_model.tflite`) 
   - Input: 2432 features (76 mel frames × 32)
   - Output: 96 embedding features
   - Purpose: Phonetic pattern recognition

3. **Wake Word** (`hey_mycroft_v0.1.tflite`)
   - Input: 1536 features (16 embeddings × 96)
   - Output: Confidence score (0.0-1.0)
   - Purpose: "Hey Mycroft" classification

### Directory Structure

```
agent-edge-rs/
├── src/
│   ├── main.rs              # Application entry point
│   ├── lib.rs               # Library exports
│   ├── error.rs             # Error handling
│   ├── audio/               # Audio capture and processing
│   │   ├── mod.rs           
│   │   ├── channel.rs       # Multi-channel extraction
│   │   └── pulse_capture.rs # PulseAudio integration
│   ├── detection/           # Detection pipeline
│   │   ├── mod.rs
│   │   └── pipeline.rs      # Main OpenWakeWord pipeline
│   ├── models/              # TensorFlow Lite model wrappers
│   │   ├── mod.rs
│   │   ├── melspectrogram.rs
│   │   ├── embedding.rs
│   │   └── wakeword.rs
│   └── vad/                 # Voice Activity Detection
│       └── mod.rs
├── models/                  # TensorFlow Lite model files
│   ├── melspectrogram.tflite
│   ├── embedding_model.tflite
│   └── hey_mycroft_v0.1.tflite
├── tests/                   # Comprehensive test suite
│   ├── data/               # Test audio files
│   ├── audio_tests.rs      # Audio processing tests
│   └── pipeline_tests.rs   # End-to-end integration test
└── openWakeWord/           # Original Python implementation (reference)
```

## 🧪 Testing

### Test Structure

- **Unit Tests** (`cargo test --lib`): 5 tests for configuration and model creation
- **Audio Tests** (`cargo test --test audio_tests`): 7 tests for channel extraction and format conversion
- **Pipeline Test** (`cargo test --test pipeline_tests`): 1 comprehensive end-to-end test

### Running Tests

```bash
# All tests (13 total)
cargo test

# Individual test suites
cargo test --lib                    # Unit tests only
cargo test --test audio_tests       # Audio processing only
cargo test --test pipeline_tests    # Integration test only

# Verbose output to see detection results
cargo test test_complete_pipeline --test pipeline_tests -- --nocapture
```

### Expected Test Results

The pipeline test validates real "Hey Mycroft" detection:

```
✅ 6a. Loaded test audio: 15232 samples (0.95s)
📏 Audio length: original 0.95s → padded 2.95s  
✅ 6b. Processed 37 chunks
📊 Detection Results:
   - Total chunks: 37
   - Detections: 1
   - Max confidence: 1.0000
   - Average confidence: 0.1364
✅ 6c. Hey Mycroft audio processing validated
🎉 All pipeline tests passed! System is working correctly.
```

## 📊 Performance

### Hardware Requirements

- **Platform**: Raspberry Pi (Pi Zero 2W minimum, Pi 3+ recommended)
- **RAM**: 50MB for models + pipeline state  
- **Audio**: 16kHz capable microphone (ReSpeaker 4-mic array recommended)
- **OS**: Raspberry Pi OS with PulseAudio

### Runtime Performance

- **CPU Usage**: 
  - With VAD: 2-5% during silence, 10-15% during speech
  - Without VAD: 15-25% continuous
- **Detection Latency**: ~1.3 seconds (due to required temporal context)
- **Memory Usage**: ~50MB RAM (fixed-size rolling windows)
- **Accuracy**: Peak confidence 1.0 on test data

### Optimization Features

- **WebRTC VAD**: Reduces CPU by 80-90% during silence
- **Static Model Loading**: Models loaded once, shared across pipeline
- **Rolling Windows**: Fixed memory usage, no unbounded growth
- **Debouncing**: 1-second cooldown prevents repeated detections

## 🔧 Configuration

### Environment Variables

```bash
# Logging (default: error)
RUST_LOG=info cargo run --release
RUST_LOG=debug cargo run --release

# VAD Type Selection (default: webrtc)
VAD_TYPE=webrtc cargo run --release    # WebRTC VAD (recommended for Pi)
VAD_TYPE=silero cargo run --release    # Silero VAD (better accuracy, higher CPU)

# VAD Sensitivity (experimental)
VAD_TRIGGER_FRAMES=3 cargo run --release    # More sensitive
VAD_SILENCE_FRAMES=10 cargo run --release   # Less sensitive
```

### Hardware Configuration

For **ReSpeaker 4-mic array** (default setup):
- Extracts channel 0 from 6-channel input
- 16kHz sample rate, S16LE format
- 50ms target latency for AEC compatibility

For **other microphones**:
- Modify `PulseAudioCaptureConfig` in `main.rs`
- Set appropriate channel count and target channel

### Performance Monitoring

Use the included performance monitoring script to compare VAD implementations:

```bash
# Compare both VAD types (automated test)
./monitor_performance.sh compare

# Monitor current process
./monitor_performance.sh monitor 60

# Test specific VAD type
./monitor_performance.sh webrtc 30
./monitor_performance.sh silero 30
```

**Expected Performance on Raspberry Pi:**
- **WebRTC VAD**: 2-5% CPU during silence, 10-15% during speech
- **Silero VAD**: 5-10% CPU during silence, 15-25% during speech (includes neural network optimization)

## 🚀 Deployment

### Deploy to Raspberry Pi

Use the deployment script to build and deploy to a Pi:

```bash
# Quick deploy (default) - only copies binary (fast for development)
./deploy-to-pi.sh myuser@192.168.1.100

# Full deploy - complete setup with models and dependencies (first time or major changes)
./deploy-to-pi.sh --full myuser@192.168.1.100
```

**Default behavior**: Quick binary-only deployment for fast development iterations.  
**Full deployment**: Use `--full` flag for first-time setup or when models/dependencies change.

The script handles ARM64 compilation, file transfer, dependency installation, and setup automatically. **Must be run from within the DevContainer** for correct ARM64 build.

### Running on the Pi

After deployment, the run script is available on your Pi:

```bash
# SSH to your Pi
ssh myuser@192.168.1.100
cd agent-edge

# Run the wake word detection
./run-agent.sh

# Or with logging
RUST_LOG=info ./run-agent.sh
```

## 🛠️ Development

### Adding New Wake Words

To detect different wake words:

1. **Train new models** using the OpenWakeWord Python toolkit
2. **Replace** `hey_mycroft_v0.1.tflite` with your new model
3. **Update** confidence thresholds in `PipelineConfig`
4. **Test** with new audio samples

### Code Style

```bash
# Format code
cargo fmt

# Check for issues
cargo clippy

# Run all tests
cargo test
```

## 🐛 Troubleshooting

### Audio Issues

```bash
# Check PulseAudio status
systemctl --user status pulseaudio

# List audio sources
pactl list sources short

# Test recording
arecord -f cd -d 2 test.wav && aplay test.wav

# Add user to audio group
sudo usermod -a -G audio $USER
```

### Model Issues

- Ensure all three `.tflite` files are in `models/` directory
- Check file permissions (readable by process)
- Verify model compatibility (OpenWakeWord v0.6.0+ format)

### Performance Issues

- Enable VAD for CPU optimization during silence
- Reduce `RUST_LOG` level in production
- Check for audio device buffer underruns
- Monitor system resources with `htop`

## 📄 License

MIT License - see LICENSE file for details.

---

**Built with** OpenWakeWord models, TensorFlow Lite, WebRTC VAD, and Rust 🦀 

## Testing

The project includes comprehensive test coverage across multiple levels:

### Unit Tests
```bash
# Run all unit tests
cargo test --lib

# Run specific module tests
cargo test stt::tests::
cargo test audio::tests::
```

### Integration Tests
```bash
# Run all tests (integration tests auto-run with API key)
export FIREWORKS_API_KEY=fw_your_key_here
cargo test

# Run only integration tests
FIREWORKS_API_KEY=fw_your_key_here cargo test --test integration_tests

# Integration tests skip gracefully without API key
cargo test --test integration_tests
# Output: "⏭️ Skipping integration test - FIREWORKS_API_KEY not found"
```

### Test Categories

1. **STT Unit Tests** (7 tests) - Core STT functionality
2. **STT State Machine Tests** (5 tests) - Speech detection patterns and timing
3. **Audio Infrastructure Tests** (8 tests) - Audio processing and format conversion
4. **Wakeword Pipeline Tests** (1 test) - End-to-end pipeline with real audio
5. **Model Loading Tests** (2 tests) - TensorFlow Lite model integration
6. **Integration Tests** (4 tests) - Full API integration with real STT service

### Transcription Verification

Integration tests use robust word-based verification:
- **Required words**: "what", "time", "is", "it" 
- **Case insensitive**: Works with any capitalization
- **Spacing tolerant**: Ignores filler words, extra punctuation
- **Examples**: "What time is it?", "um what time is it", "WHAT TIME IS IT???"

### API Key Requirements

Integration tests require a valid Fireworks AI API key:
- Set `FIREWORKS_API_KEY=fw_your_key_here` in environment
- Or add to `.env` file: `FIREWORKS_API_KEY=fw_your_key_here`
- Tests automatically skip without API key (no failures)
- With API key, tests make real API calls and consume credits

## Setup

1. Install Rust and Cargo
2. Clone the repository
3. Set up environment variables (optional for integration tests)
4. Run tests: `cargo test`

## Development

The codebase is structured for modularity and testability:
- All major components have unit tests
- Integration tests verify end-to-end functionality
- Tests run in parallel for fast feedback
- No test overlap - each suite covers unique functionality 