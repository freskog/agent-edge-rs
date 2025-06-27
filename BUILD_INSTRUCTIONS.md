# Build Instructions

## Platform-Specific Audio Backends

The system automatically uses the right audio backend for each platform:

- **Linux**: PulseAudio (always)
- **macOS**: CPAL (always)  
- **Windows**: CPAL (always)

## Quick Start

### Build for any platform
```bash
# Simple - just build and it will work correctly for your platform
cargo build

# Run the audio demo
cargo run --bin audio-demo
```

### LED Ring Support

LED ring control is **enabled by default** but only works on Linux with USB access:

```bash
# Default build (includes LED ring support)
cargo build

# Build without LED ring support (smaller binary, no USB dependencies)
cargo build --no-default-features

# Explicitly enable LED ring support
cargo build --features led_ring
```

## Platform Behavior

### Linux 🐧
- **Audio**: PulseAudio backend (requires `libpulse-dev`)
- **LED Ring**: Enabled by default (requires `libusb`)
- **Dependencies**: 
  ```bash
  # Ubuntu/Debian
  sudo apt-get install libpulse-dev
  ```

### macOS 🍎  
- **Audio**: CPAL backend using Core Audio
- **LED Ring**: Compiles to stub (safe no-op functions)
- **Dependencies**: None needed

### Windows 🪟
- **Audio**: CPAL backend using WASAPI  
- **LED Ring**: Compiles to stub (safe no-op functions)
- **Dependencies**: None needed

## Testing

```bash
# Test audio capture
cargo run --bin audio-demo

# Test without LED ring 
cargo run --bin audio-demo --no-default-features
```

## Feature Flags

| Feature | Description | Default |
|---------|-------------|---------|
| `led_ring` | USB LED ring control | Yes |

That's it! No complex feature selection needed - the platform detection happens automatically at compile time. 