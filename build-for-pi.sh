#!/usr/bin/env bash
set -e

DEPLOY_DIR="deploy-pi"

echo "🔧 Building for Raspberry Pi (container build)..."

# Build for aarch64 (64-bit Raspberry Pi)
echo "📦 Installing target..."
rustup target add aarch64-unknown-linux-gnu

echo "🔨 Building release binary..."
cargo build --release --target aarch64-unknown-linux-gnu

# Find the built TensorFlow Lite library
TFLITE_LIB=$(find target -name "libtensorflowlite_c.so" -path "*aarch64*" -path "*release*" | head -1)

if [ -z "$TFLITE_LIB" ]; then
    echo "❌ Could not find libtensorflowlite_c.so for aarch64"
    echo "Make sure the build completed successfully"
    exit 1
fi

BINARY="target/aarch64-unknown-linux-gnu/release/agent-edge"

if [ ! -f "$BINARY" ]; then
    echo "❌ Binary not found: $BINARY"
    exit 1
fi

echo "✅ Found library: $TFLITE_LIB"
echo "✅ Found binary: $BINARY"

# Create deployment directory
rm -rf "$DEPLOY_DIR"
mkdir -p "$DEPLOY_DIR/lib"

# Copy binary and library
cp "$BINARY" "$DEPLOY_DIR/"
cp "$TFLITE_LIB" "$DEPLOY_DIR/lib/"

# Create run script
cat > "$DEPLOY_DIR/run-agent.sh" << 'EOF'
#!/bin/bash
# Set library path and run the agent
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LD_LIBRARY_PATH="$DIR/lib:$LD_LIBRARY_PATH" exec "$DIR/agent-edge" "$@"
EOF

chmod +x "$DEPLOY_DIR/run-agent.sh"

echo "📦 Deployment package created in $DEPLOY_DIR/"
echo ""
echo "📁 Package contents:"
ls -la "$DEPLOY_DIR/"
echo ""
echo "📊 Sizes:"
echo "   Binary: $(ls -lh "$DEPLOY_DIR/agent-edge" | awk '{print $5}')"
echo "   Library: $(ls -lh "$DEPLOY_DIR/lib/libtensorflowlite_c.so" | awk '{print $5}')"
echo ""
echo "🎯 Ready for deployment from host machine!"
echo "   Run './deploy-from-host.sh' from your Mac to transfer to Pi" 