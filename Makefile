# Rust Agent Edge Testing Makefile

.PHONY: test test-basic test-audio test-api test-full test-integration help

# Default test - runs basic tests that don't require external dependencies
test: test-basic

# Basic tests only (no API keys or audio devices required)
test-basic:
	@echo "🧪 Running basic tests (no external dependencies required)..."
	cargo test --lib

# Tests requiring audio devices
test-audio:
	@echo "🔊 Running tests that require audio devices..."
	cargo test --lib --features test-audio

# Tests requiring API keys
test-api:
	@echo "🔑 Running tests that require API keys..."
	cargo test --lib --features test-api

# Full test suite (requires both API keys and audio devices)
test-full:
	@echo "🚀 Running full test suite (requires API keys and audio devices)..."
	cargo test --lib --features test-api,test-audio

# Integration tests (includes pipeline tests)
test-integration:
	@echo "⚙️  Running integration tests..."
	cargo test --features test-integration

# All tests including integration
test-all:
	@echo "🎯 Running all tests..."
	cargo test --features test-api,test-audio,test-integration

# Check what test capabilities are available
test-check:
	@echo "🔍 Checking test environment capabilities..."
	cargo test test_environment_capabilities -- --nocapture

# Help target
help:
	@echo "Available test targets:"
	@echo "  test-basic       - Basic tests (default, no external dependencies)"
	@echo "  test-audio       - Tests requiring audio devices"
	@echo "  test-api         - Tests requiring API keys (FIREWORKS_API_KEY)"
	@echo "  test-full        - Full test suite (API keys + audio devices)"
	@echo "  test-integration - Integration tests"
	@echo "  test-all         - All tests including integration"
	@echo "  test-check       - Check what test capabilities are available"
	@echo ""
	@echo "Environment variables:"
	@echo "  FIREWORKS_API_KEY - Required for API-dependent tests"
	@echo ""
	@echo "Examples:"
	@echo "  make test-check                    # Check what's available"
	@echo "  FIREWORKS_API_KEY=xxx make test-api  # Run API tests"
	@echo "  make test-full                     # Run everything if env is set up" 