# Agent Edge RS - Wakeword Detection System

A Rust-based wakeword detection system using OpenWakeWord models for real-time audio processing and keyword spotting.

## 🎯 Project Overview

This project provides a complete wakeword detection solution optimized for edge devices, featuring:

- **Real-time Audio Processing**: Live microphone capture with PulseAudio integration
- **OpenWakeWord Detection**: 3-stage pipeline for accurate "hey mycroft" detection
- **Voice Activity Detection (VAD)**: WebRTC VAD for CPU optimization during silence
- **Edge Optimization**: Designed for low-power devices like Raspberry Pi

## 🚀 Quick Start

### Option 1: DevContainer (Recommended)

The easiest way to get started is using VS Code with the DevContainer extension:

1. **Prerequisites**: Install [VS Code](https://code.visualstudio.com/) and the [Dev Containers extension](https://marketplace.visualstudio.com/items?itemName=ms-vscode-remote.remote-containers)

2. **Clone and open**:
   ```bash
   git clone <repository-url>
   code agent-edge-rs
   ```

3. **Reopen in container**: VS Code will prompt to "Reopen in Container" - click yes

4. **Build and run**:
   ```bash
   # Build the project
   cargo build --release --features pulse
   
   # Run the wakeword detection system
   cargo run --release --features pulse
   ```

### Option 2: Docker

If you prefer Docker directly:

1. **Build the container**:
   ```bash
   docker build -t agent-edge-rs .
   ```

2. **Run with audio access**:
   ```bash
   # Run with PulseAudio socket access (Linux)
   docker run --rm -it \
     --device /dev/snd \
     -v /run/user/$(id -u)/pulse:/run/user/1000/pulse \
     -e PULSE_RUNTIME_PATH=/run/user/1000/pulse \
     agent-edge-rs
   ```

## 🎤 Usage

Once running, the system will:

1. **Initialize** the OpenWakeWord 3-stage detection pipeline
2. **Enable WebRTC VAD** for CPU optimization during silence
3. **Start listening** for the wakeword "hey mycroft"
4. **Display status** updates every few seconds
5. **Alert** when the wakeword is detected

### Environment Variables

- `VAD_TRIGGER_FRAMES`: Custom VAD trigger sensitivity
- `VAD_SILENCE_FRAMES`: Custom VAD silence detection frames

## 🏗️ Architecture

### Core Components

```
src/
├── main.rs              # Main application entry point
├── lib.rs               # Library exports
├── error.rs             # Error handling
├── audio/
│   ├── mod.rs           # Audio module
│   ├── channel.rs       # Audio channel management
│   └── pulse_capture.rs # PulseAudio integration
├── detection/
│   ├── mod.rs           # Detection module
│   └── pipeline.rs      # OpenWakeWord pipeline
├── models/
│   ├── mod.rs           # Model loading
│   ├── embedding.rs     # Embedding model
│   ├── melspectrogram.rs # Mel-spectrogram preprocessing
│   └── wakeword.rs      # Wakeword detection model
└── vad/
    └── mod.rs           # Voice Activity Detection
```

### Models

The system uses three TensorFlow Lite models in sequence:

1. **`melspectrogram.tflite`** - Audio preprocessing
2. **`embedding_model.tflite`** - Feature extraction
3. **`hey_mycroft_v0.1.tflite`** - Wakeword classification

## 🔧 Configuration

### VAD Tuning

For environments with different noise levels:

```bash
# More sensitive VAD (triggers faster)
VAD_TRIGGER_FRAMES=3 cargo run --release --features pulse

# Less sensitive VAD (requires more silence)
VAD_SILENCE_FRAMES=50 cargo run --release --features pulse
```

### Audio Troubleshooting

If audio capture fails:

1. **Check PulseAudio status**: `systemctl --user status pulseaudio`
2. **Start PulseAudio**: `pulseaudio --start`
3. **List audio devices**: `pactl list sources short`
4. **Test recording**: `arecord -f cd -d 1 test.wav`
5. **Add to audio group**: `sudo usermod -a -G audio $USER`

## 📊 Performance

- **CPU Usage**: ~2-5% on modern hardware with VAD enabled
- **Memory Usage**: ~50MB RAM
- **Detection Latency**: <200ms from audio to detection
- **Accuracy**: Optimized for "hey mycroft" with minimal false positives

## 🚀 Deployment

### Raspberry Pi Deployment

The `deploy-pi/` directory contains deployment artifacts:

- `agent-edge` - Compiled binary
- `lib/` - Required libraries
- `run-agent.sh` - Startup script
- `models/` - Required model files (symlinked to main models/)

## 🤝 Contributing

### Development Setup

1. **Clone the repository**
2. **Install dependencies** (see Prerequisites)
3. **Build with PulseAudio support**: `cargo build --features pulse`
4. **Run tests**: `cargo test`

### Code Style

- Follow standard Rust conventions
- Use `cargo fmt` for formatting
- Run `cargo clippy` for linting
- Add documentation for public APIs

## 📄 License

This project is licensed under the MIT License - see the LICENSE file for details.

---

**Note**: This system is optimized for the "hey mycroft" wakeword. For other wakewords, you'll need to train and provide different models for the final classification stage. 