# Testing Guide for Agent Edge RS

This project uses a categorized testing approach to handle different types of tests that may require external dependencies.

## Test Categories

### 🧪 Basic Tests (Default)
Tests that run without any external dependencies - no API keys or audio devices required.

```bash
cargo test --lib
# or
make test-basic
```

### 🔊 Audio Tests
Tests that require audio input devices to be available.

```bash
cargo test --lib --features test-audio
# or
make test-audio
```

### 🔑 API Tests
Tests that require API keys (specifically `FIREWORKS_API_KEY` environment variable).

```bash
FIREWORKS_API_KEY=your_key cargo test --lib --features test-api
# or
FIREWORKS_API_KEY=your_key make test-api
```

### 🚀 Full Test Suite
Tests that require both API keys and audio devices.

```bash
FIREWORKS_API_KEY=your_key cargo test --lib --features test-api,test-audio
# or
FIREWORKS_API_KEY=your_key make test-full
```

### ⚙️ Integration Tests
Comprehensive end-to-end tests including pipeline validation.

```bash
cargo test --features test-integration
# or
make test-integration
```

## Environment Check

To see what testing capabilities are available in your current environment:

```bash
cargo test test_environment_capabilities -- --nocapture
# or
make test-check
```

This will output something like:
```
🔍 Environment Capabilities:
  - API Key (FIREWORKS_API_KEY): ❌ Missing
  - Audio Device: ❌ Missing
💡 To run full tests:
  - Set FIREWORKS_API_KEY environment variable
  - Ensure audio input device is available
  - Run: cargo test --features test-api,test-audio
```

## Test Behavior

### ✅ What Happens Now
- **Tests are properly categorized** and only run when their dependencies are available
- **Missing dependencies cause tests to be ignored** rather than silently skipped
- **Clear error messages** indicate what's required to run each test category
- **Environment check** tells you exactly what's available and how to run more tests

### ❌ What Used to Happen
- Tests would silently skip with early returns, appearing to "pass" when they didn't actually run
- No clear indication of what was required to make tests run
- Difficult to distinguish between "test passed" and "test was skipped"

## Running Tests in CI/CD

### GitHub Actions Example
```yaml
- name: Run basic tests
  run: cargo test --lib

- name: Run API tests (if API key available)
  run: cargo test --lib --features test-api
  env:
    FIREWORKS_API_KEY: ${{ secrets.FIREWORKS_API_KEY }}
  if: env.FIREWORKS_API_KEY != ''

- name: Run audio tests (Linux with virtual audio)
  run: |
    # Set up virtual audio device for CI
    sudo modprobe snd-dummy
    cargo test --lib --features test-audio
```

## Test Structure

```
tests/
├── Basic Tests (always run)
│   ├── Configuration validation
│   ├── Data structure tests
│   └── Unit tests without external deps
│
├── Audio Tests (feature: test-audio)
│   ├── Audio device detection
│   ├── Audio capture functionality
│   └── Audio processing pipelines
│
├── API Tests (feature: test-api)
│   ├── STT service integration
│   ├── TTS service integration
│   └── LLM service integration
│
└── Integration Tests (feature: test-integration)
    ├── End-to-end pipeline tests
    ├── Real audio file processing
    └── Complete workflow validation
```

## Best Practices

1. **Always write basic tests first** - ensure core logic works without external dependencies
2. **Use feature flags** to clearly indicate test requirements
3. **Provide helpful error messages** when requirements aren't met
4. **Document what each test requires** in comments or test names
5. **Use the environment check** to validate your test setup

## Quick Commands

```bash
# Check what's available
make test-check

# Run basic tests (always works)
make test-basic

# Run everything if environment is set up
FIREWORKS_API_KEY=your_key make test-all

# Get help on available commands
make help
``` 