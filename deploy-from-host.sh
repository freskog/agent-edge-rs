#!/usr/bin/env bash
set -e

# Configuration
PI_HOST="freskog@10.10.100.98"
DEPLOY_DIR="deploy-pi"

echo "🚀 Deploying to Raspberry Pi from host machine..."

# Check if deployment directory exists
if [ ! -d "$DEPLOY_DIR" ]; then
    echo "❌ Deployment directory not found."
    echo "   Run './build-for-pi.sh' inside the Docker container first."
    exit 1
fi

# Check if required files exist
if [ ! -f "$DEPLOY_DIR/agent-edge" ]; then
    echo "❌ Binary not found in $DEPLOY_DIR/"
    echo "   Run './build-for-pi.sh' inside the Docker container first."
    exit 1
fi

if [ ! -f "$DEPLOY_DIR/lib/libtensorflowlite_c.so" ]; then
    echo "❌ TensorFlow Lite library not found in $DEPLOY_DIR/"
    echo "   Run './build-for-pi.sh' inside the Docker container first."
    exit 1
fi

echo "📁 Deployment package contents:"
ls -la "$DEPLOY_DIR/"
echo ""

# Test SSH connection
echo "🔍 Testing SSH connection..."
if ! ssh -o ConnectTimeout=5 -o BatchMode=yes "$PI_HOST" exit 2>/dev/null; then
    echo "⚠️  SSH keys not set up, will prompt for password..."
    echo "   To set up keys: ssh-copy-id $PI_HOST"
fi

# Transfer files
echo "📤 Transferring files to Pi..."
rsync -av --progress "$DEPLOY_DIR/" "$PI_HOST:~/agent-edge/"

# Test deployment
echo ""
echo "🧪 Testing deployment on Pi..."
ssh "$PI_HOST" << 'EOSSH'
    echo "📋 System info:"
    echo "   OS: $(cat /etc/os-release | grep PRETTY_NAME | cut -d'"' -f2)"
    echo "   Arch: $(uname -m)"
    echo "   Kernel: $(uname -r)"
    echo ""
    
    cd ~/agent-edge
    
    echo "🔍 Checking files..."
    ls -la
    echo ""
    
    echo "🔍 Checking library dependencies..."
    if command -v ldd &> /dev/null; then
        echo "Binary dependencies:"
        ldd ./agent-edge | head -10
        echo ""
        echo "Library dependencies:"
        ldd ./lib/libtensorflowlite_c.so | head -5
    else
        echo "ldd not available"
    fi
    echo ""
    
    echo "🏃 Testing agent..."
    if ./run-agent.sh --help > /dev/null 2>&1; then
        echo "✅ Agent runs successfully!"
        echo ""
        echo "📋 Available options:"
        ./run-agent.sh --help
    else
        echo "❌ Agent failed to run"
        echo "Trying to run directly for debugging:"
        LD_LIBRARY_PATH="./lib:$LD_LIBRARY_PATH" ./agent-edge --help || true
    fi
EOSSH

echo ""
echo "🎉 Deployment complete!"
echo ""
echo "💡 To run on the Pi:"
echo "   ssh $PI_HOST"
echo "   cd ~/agent-edge"
echo "   ./run-agent.sh --help"
echo ""
echo "🔧 To start the agent:"
echo "   ./run-agent.sh [options]" 