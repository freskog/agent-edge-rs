# Agent Edge RS

A **Linux-only** wakeword detection system optimized for Raspberry Pi edge devices. Cross-compile from macOS/Windows, deploy to Linux.

## Features

- **🎯 Embedded Wakeword Detection**: OpenWakeWord "hey mycroft" with TensorFlow Lite
- **🔊 Low-Latency Audio**: 50ms PulseAudio capture for AEC compatibility  
- **⚡ Edge Optimized**: Single-core Raspberry Pi 3+ ready
- **🏗️ Cross-Compilation**: Develop anywhere, deploy to Linux
- **📦 Single Binary**: Models embedded, no external dependencies

## Architecture

```
┌─────────────────┐    ┌──────────────────┐    ┌─────────────────┐
│   Audio Input   │───▶│  Mel Processor   │───▶│ Wakeword Model  │
│  (80ms chunks)  │    │ (melspectrogram) │    │ (hey_mycroft)   │
└─────────────────┘    └──────────────────┘    └─────────────────┘
        │                       │                       │
        ▼                       ▼                       ▼
  16kHz F32LE              80 mel features      Confidence score
   1280 samples             per 80ms chunk        (0.0 - 1.0)
```

## Quick Start

### Building on Raspberry Pi (Recommended)

The TensorFlow Lite dependencies are complex to cross-compile, so we recommend building directly on your Raspberry Pi:

```bash
# On your Raspberry Pi
git clone <your-repo>
cd agent-edge-rs

# Install dependencies
sudo apt update
sudo apt install -y pulseaudio-utils libpulse-dev pkg-config build-essential

# Build (this may take 5-10 minutes on Pi 4, longer on Pi 3)
cargo build --release

# Test
./target/release/agent-edge --verbose
```

### Cross-Compilation (Advanced)

Cross-compilation from macOS/Windows to Linux ARM is complex due to TensorFlow Lite's build requirements. If you need to cross-compile, consider:

1. **Using a Docker-based build environment** on a Linux machine
2. **Building on CI/CD runners** with appropriate Linux ARM environments  
3. **Using GitHub Actions** with cross-compilation setup

For development iteration, we recommend the direct build approach above.

## Development Workflow

### Local Development (macOS/Linux)

You can develop and test the audio/detection logic locally, but **note that TensorFlow Lite models will only work on the target platform**:

```bash
# Local development - models will not load, but you can test structure
cargo check
cargo test

# Deploy to Pi for testing  
git push origin main
# Then on Pi: git pull && cargo build --release
```

### Testing Strategy

- **Unit tests**: Run locally with `cargo test` (models are mocked/stubbed)
- **Integration tests**: Run on actual Raspberry Pi hardware
- **Audio testing**: Use Pi with actual microphone hardware

## CLI Options

```bash
agent-edge [OPTIONS]

Options:
  -v, --verbose                Enable verbose debug logging
      --device <NAME>          Use specific PulseAudio device name
      --threshold <FLOAT>      Wakeword confidence threshold (0.0-1.0) [default: 0.8]
      --latency <MS>           Target audio latency in milliseconds [default: 50]
  -h, --help                   Print help
```

## Requirements

### Runtime (Raspberry Pi)
- **OS**: 64-bit Raspberry Pi OS (Bullseye+)
- **Hardware**: Raspberry Pi 3+ with ReSpeaker 4-mic USB array
- **Audio**: PulseAudio installed and running
- **Memory**: ~100MB RAM for wakeword detection

### Development (Cross-compilation)
- **Rust**: 1.70+ with `aarch64-unknown-linux-gnu` target
- **Cross**: `cargo install cross` (Docker-based)

## Audio Pipeline

### Input Requirements
- **Sample Rate**: 16kHz
- **Format**: F32LE (32-bit float)
- **Channels**: 6-channel ReSpeaker (uses channel 0)
- **Chunk Size**: 80ms (1280 samples)
- **Latency**: 50ms (AEC compatible)

### Performance Targets
- **Raspberry Pi 4**: <25% CPU, <100MB RAM, <60ms latency
- **Raspberry Pi 3**: <50% CPU, <150MB RAM, <100ms latency

## Models

### Embedded TensorFlow Lite
- **`melspectrogram.tflite`**: Audio → 80 mel features (80ms chunks)
- **`hey_mycroft_v0.1.tflite`**: 76 frames → confidence score
- **Format**: OpenWakeWord v0.5.0+ compatible
- **Deployment**: Embedded in binary, no external files

### Detection Pipeline
1. **Audio Capture**: 6-channel → channel 0 extraction
2. **Mel Processing**: 1280 samples → 80 mel features  
3. **Frame Buffering**: Accumulate 76 frames (~6 seconds)
4. **Wakeword Detection**: Confidence score + threshold check

## Cross-Compilation Setup

### One-time Setup
```bash
# Install Rust target
rustup target add aarch64-unknown-linux-gnu

# Install cross (Docker-based cross-compilation)
cargo install cross

# Verify Docker is running
docker info
```

### Build Commands
```bash
# Development build (faster)
cross build --target aarch64-unknown-linux-gnu

# Production build (optimized)
cross build --target aarch64-unknown-linux-gnu --release

# Check what was built
ls -la target/aarch64-unknown-linux-gnu/release/agent-edge
```

## Deployment

### Transfer to Raspberry Pi
```bash
# Copy binary
scp target/aarch64-unknown-linux-gnu/release/agent-edge pi@raspberrypi.local:~/

# Copy with executable permissions
ssh pi@raspberrypi.local 'chmod +x agent-edge'
```

### ReSpeaker Setup (Pi)
```bash
# Install ReSpeaker drivers (if needed)
sudo apt update
sudo apt install pulseaudio pulseaudio-utils

# Verify ReSpeaker detection
lsusb | grep -i seeed
pactl list sources short

# Test audio capture (press Ctrl+C to stop)
./agent-edge --verbose
```

## Example Output

```
[INFO] 🚀 Starting agent-edge wakeword detection
[INFO]    Platform: aarch64 on linux
[INFO] Initializing wakeword detection pipeline...
[INFO] MelSpectrogram processor initialized:
[INFO]   - Chunk duration: 80ms (1280 samples at 16000Hz)
[INFO]   - Mel bins: 80
[INFO] Wakeword detector initialized:
[INFO]   - Frame window size: 76
[INFO]   - Confidence threshold: 0.50
[INFO] 🎤 Starting audio capture...
[INFO]    Chunk size: 1280 samples (80ms)
[INFO]    Target latency: 50ms
[INFO]    Listening for wakeword 'hey mycroft'...
[INFO] 🎯 WAKEWORD DETECTED!
[INFO]    Confidence: 0.847
[INFO]    Frame: 123
```

## Troubleshooting

### Audio Issues
```bash
# Check PulseAudio status
pulseaudio --check -v

# List audio devices
pactl list sources short

# Test with specific device
./agent-edge --device "ReSpeaker_4_mic_array"
```

### Cross-Compilation Issues
```bash
# Clean build cache
cross clean

# Verify target is installed  
rustup target list --installed | grep aarch64

# Test Docker
docker run --rm hello-world
```

### Performance Issues
```bash
# Monitor CPU usage
htop

# Monitor with verbose logging
./agent-edge --verbose

# Lower threshold for more detections  
./agent-edge --threshold 0.6
```

## License

Apache 2.0 - See [LICENSE](LICENSE) for details. 