# Agent Edge RS - Edge AI with TensorFlow Lite

A Rust-based edge AI agent for real-time audio processing and wakeword detection, featuring comprehensive TensorFlow Lite integration and operation compatibility testing.

## 🎯 Project Overview

This project provides a complete solution for running TensorFlow Lite models on edge devices, with special focus on:

- **Audio Processing**: Real-time audio capture and preprocessing
- **Wakeword Detection**: Using TensorFlow Lite models for keyword spotting
- **Operation Compatibility**: Comprehensive testing of TensorFlow Lite operation support
- **Edge Deployment**: Optimized for Raspberry Pi and similar edge devices

## 🔍 TensorFlow Lite Operation 126 Compatibility Issue

### Problem Identified
The project discovered a critical compatibility issue:
- ✅ **hey_mycroft_v0.1.tflite** (860KB) - Works perfectly with tflite crate v0.9.8
- ❌ **melspectrogram.tflite** (1.09MB) - Fails with "Op builtin_code out of range: 126"

### Root Cause
Operation 126 is not supported in the Rust `tflite` crate v0.9.8, which is currently the latest available version on crates.io.

### Solutions Implemented

#### 1. Latest TensorFlow Lite C Library Installation
The project includes automatic installation of the latest TensorFlow C library (v2.18.1):

```bash
# Automatically downloads and installs the latest TensorFlow C library
cargo run  # Includes latest TensorFlow Lite test
```

#### 2. Working Model Integration
Uses compatible models while providing the infrastructure for future upgrades:

```rust
// Working implementation with hey_mycroft model
let processor = WorkingMelSpectrogramProcessor::new(config)?;
let result = processor.process_audio_chunk(&audio_data)?;
```

## 🚀 Quick Start

### Prerequisites

#### Development Environment (DevContainer)
The project includes a complete DevContainer setup:

```bash
# Clone and open in VS Code with DevContainer extension
git clone <repository-url>
code agent-edge-rs
# VS Code will prompt to reopen in container
```

#### Manual Installation (Raspberry Pi OS Lite)

```bash
# Install system dependencies
sudo apt-get update && sudo apt-get install -y \
    build-essential \
    gcc \
    g++ \
    pkg-config \
    libssl-dev \
    git \
    libasound2-dev \
    libpulse-dev \
    tar \
    gzip \
    wget \
    ca-certificates

# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source ~/.cargo/env
```

### Running the Tests

```bash
# Run comprehensive TensorFlow Lite compatibility tests
cargo run

# Run specific tests
cargo test

# Run with detailed logging
RUST_LOG=debug cargo run
```

## 🧪 Test Suite

The project includes comprehensive testing for TensorFlow Lite compatibility:

### 1. Solution Analysis
- Comprehensive analysis of the operation 126 compatibility issue
- Detailed breakdown of available solutions
- Performance and compatibility metrics

### 2. Working Model Test
- Tests with compatible hey_mycroft model
- Demonstrates working thread-safe TensorFlow Lite integration
- Validates audio processing pipeline

### 3. Latest TensorFlow Lite C Library Test
- **Downloads and installs TensorFlow C library v2.18.1**
- **Tests C compilation and linking**
- **Prepares infrastructure for custom Rust bindings**

### 4. Model Inspector
- Detailed analysis of model structure
- Operation enumeration and compatibility checking
- Input/output tensor analysis

## 📊 Compatibility Matrix

| Model | Size | tflite v0.9.8 | TensorFlow C v2.18.1 | Status |
|-------|------|---------------|----------------------|--------|
| hey_mycroft_v0.1.tflite | 860KB | ✅ Works | ✅ Compatible | Production Ready |
| melspectrogram.tflite | 1.09MB | ❌ Op 126 Error | 🔧 Requires Custom Bindings | In Progress |

## 🛠️ Architecture

### Core Components

```
src/
├── audio/          # Audio capture and processing
├── models/         # TensorFlow Lite model integration
│   ├── simple_thread_local.rs    # Thread-safe implementation
│   ├── working_melspec.rs        # Compatible model processor
│   ├── latest_tflite_test.rs     # Latest C library testing
│   └── solution_summary.rs       # Comprehensive analysis
├── error/          # Error handling
└── main.rs         # Test runner and demonstration
```

### Key Features

- **Thread-Safe Processing**: Cached model metadata with safe concurrent access
- **Comprehensive Error Handling**: Detailed error reporting and diagnostics
- **Automatic Dependency Management**: Downloads and installs latest TensorFlow C library
- **Cross-Platform Support**: Works on x86_64 and ARM64 (Raspberry Pi)

## 🔧 Development Workflow

### Adding New Models

1. **Place model file** in `models/` directory
2. **Run compatibility test**:
   ```bash
   cargo run  # Includes model inspection
   ```
3. **Check operation support** in test output
4. **Use working patterns** from existing implementations

### Custom Operation Support

For models requiring unsupported operations:

1. **Install latest TensorFlow C library** (automated):
   ```rust
   models::latest_tflite_test::run_comprehensive_latest_test()?;
   ```

2. **Build custom Rust bindings** using installed library
3. **Integrate with existing architecture**

## 📋 Current Status

### ✅ Working Features
- Audio capture and processing pipeline
- Thread-safe TensorFlow Lite integration
- hey_mycroft wakeword detection model
- Comprehensive compatibility testing
- Latest TensorFlow C library installation
- Cross-platform DevContainer support

### 🔧 In Progress
- Custom Rust bindings for operation 126 support
- melspectrogram model integration
- Performance optimization for edge devices

### 🎯 Roadmap
- [ ] Complete operation 126 support
- [ ] Edge-optimized audio preprocessing
- [ ] Multi-model inference pipeline
- [ ] Hardware acceleration (GPU/NPU)
- [ ] Production deployment tools

## 🤝 Contributing

### Development Setup

1. **Use DevContainer** for consistent environment
2. **Run tests** before submitting changes:
   ```bash
   cargo test
   cargo run  # Integration tests
   ```
3. **Follow Rust conventions** and add documentation

### Reporting Issues

When reporting TensorFlow Lite compatibility issues:

1. **Include model details** (size, source, operations)
2. **Run diagnostic tests**:
   ```bash
   RUST_LOG=debug cargo run
   ```
3. **Provide complete error output**

## 📚 Additional Resources

- [TensorFlow Lite Operations](https://www.tensorflow.org/lite/guide/ops_compatibility)
- [TensorFlow C API Documentation](https://www.tensorflow.org/install/lang_c)
- [Rust TensorFlow Lite Crate](https://crates.io/crates/tflite)
- [Edge AI Best Practices](https://www.tensorflow.org/lite/performance/best_practices)

## 📄 License

This project is licensed under the MIT License - see the LICENSE file for details.

---

**Note**: This project actively addresses TensorFlow Lite operation compatibility issues and provides a complete framework for edge AI development. The latest TensorFlow C library installation ensures compatibility with the newest operations and models. 