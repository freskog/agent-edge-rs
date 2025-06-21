#!/bin/bash

# Set the library path for TensorFlow Lite
TFLITE_LIB_PATH="./target/release/build/tflitec-5cab709581ba9805/out"

# Check if the library exists
if [ ! -f "$TFLITE_LIB_PATH/libtensorflowlite_c.so" ]; then
    echo "Error: TensorFlow Lite library not found at $TFLITE_LIB_PATH"
    echo "Please run 'cargo build --release' first"
    exit 1
fi

# Check if the binary exists
if [ ! -f "./target/release/agent-edge" ]; then
    echo "Error: Release binary not found at ./target/release/agent-edge"
    echo "Please run 'cargo build --release' first"
    exit 1
fi

echo "🚀 Starting Wakeword Detection System..."
echo "💡 Tip: You can control logging with RUST_LOG environment variable"
echo "   Examples:"
echo "     RUST_LOG=info ./run-agent-release.sh    (normal output)"
echo "     RUST_LOG=debug ./run-agent-release.sh   (verbose output)"
echo "     RUST_LOG=warn ./run-agent-release.sh    (minimal output)"
echo ""

# Set environment and run with default logging if not specified
export LD_LIBRARY_PATH="$TFLITE_LIB_PATH:$LD_LIBRARY_PATH"
export RUST_LOG="${RUST_LOG:-info}"  # Default to info level if not set
exec ./target/release/agent-edge "$@" 