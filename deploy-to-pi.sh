#!/bin/bash

# Deploy Agent Edge to Raspberry Pi
# NOTE: Run this from within the DevContainer for correct ARM64 build
# Usage: ./deploy-to-pi.sh user@hostname-or-ip

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

if [ $# -eq 0 ]; then
    echo -e "${RED}Usage: $0 <user@hostname-or-ip>${NC}"
    echo "Example: $0 pi@192.168.1.100"
    echo "         $0 myuser@raspberrypi.local"
    echo "         $0 admin@10.0.1.50"
    exit 1
fi

PI_TARGET="$1"
PROJECT_NAME="agent-edge"

# Extract username from target (user@host format)
if [[ "$PI_TARGET" == *"@"* ]]; then
    PI_USER="${PI_TARGET%@*}"
    PI_HOST="${PI_TARGET#*@}"
else
    echo -e "${RED}❌ Please use user@hostname format (e.g., myuser@192.168.1.100)${NC}"
    exit 1
fi

REMOTE_DIR="/home/${PI_USER}/${PROJECT_NAME}"

echo -e "${BLUE}🚀 Deploying Agent Edge to Raspberry Pi${NC}"
echo -e "${BLUE}===========================================${NC}"
echo "Target: $PI_TARGET"
echo "User: $PI_USER"
echo "Host: $PI_HOST"
echo "Remote directory: $REMOTE_DIR"
echo ""

# Check if we have the required tools
echo -e "${YELLOW}🔧 Checking build environment...${NC}"

if ! command -v cargo &> /dev/null; then
    echo -e "${RED}❌ Cargo not found. Please install Rust.${NC}"
    exit 1
fi

# Note: This script should be run from within the DevContainer
echo -e "${YELLOW}ℹ️  Building for ARM64 (run from DevContainer)...${NC}"

# Check if models exist
echo -e "${YELLOW}📦 Checking models...${NC}"
if [ ! -d "models" ]; then
    echo -e "${RED}❌ Models directory not found${NC}"
    echo "Please ensure the models/ directory exists with:"
    echo "  - melspectrogram.tflite"
    echo "  - embedding_model.tflite"  
    echo "  - hey_mycroft_v0.1.tflite"
    exit 1
fi

REQUIRED_MODELS=(
    "models/melspectrogram.tflite"
    "models/embedding_model.tflite"
    "models/hey_mycroft_v0.1.tflite"
)

for model in "${REQUIRED_MODELS[@]}"; do
    if [ ! -f "$model" ]; then
        echo -e "${RED}❌ Required model not found: $model${NC}"
        exit 1
    fi
done

echo -e "${GREEN}✅ All models found${NC}"

# Build for ARM64 (assumes DevContainer environment)
echo -e "${YELLOW}🔨 Building for ARM64...${NC}"
cargo build --release

if [ ! -f "target/release/agent-edge-rs" ]; then
    echo -e "${RED}❌ Build failed${NC}"
    echo "Make sure you're running this from within the DevContainer"
    exit 1
fi

echo -e "${GREEN}✅ Build successful${NC}"

# Test SSH connection
echo -e "${YELLOW}🌐 Testing SSH connection...${NC}"
if ! ssh -o ConnectTimeout=5 "$PI_TARGET" "echo 'SSH connection successful'" 2>/dev/null; then
    echo -e "${RED}❌ Cannot connect to $PI_TARGET${NC}"
    echo "Please check:"
    echo "  - Pi is powered on and connected to network"
    echo "  - SSH is enabled on the Pi"
    echo "  - Hostname/IP address is correct"
    echo "  - SSH keys are set up (run: ssh-copy-id $PI_TARGET)"
    exit 1
fi

echo -e "${GREEN}✅ SSH connection successful${NC}"

# Create remote directory structure
echo -e "${YELLOW}📁 Setting up remote directories...${NC}"
ssh "$PI_TARGET" "mkdir -p $REMOTE_DIR/{lib,models}"

# Copy files to Pi
echo -e "${YELLOW}📤 Uploading files...${NC}"

# Copy binary
echo "  - Binary..."
scp target/release/agent-edge-rs "$PI_TARGET:$REMOTE_DIR/"

# Copy models
echo "  - Models..."
scp -r models/ "$PI_TARGET:$REMOTE_DIR/"

# Copy run script
echo "  - Run script..."
scp run-agent.sh "$PI_TARGET:$REMOTE_DIR/"

# Make scripts executable on remote
ssh "$PI_TARGET" "chmod +x $REMOTE_DIR/agent-edge-rs $REMOTE_DIR/run-agent.sh"

echo -e "${GREEN}✅ Files uploaded successfully${NC}"

# Install dependencies on Pi
echo -e "${YELLOW}🔧 Installing dependencies on Pi...${NC}"
ssh "$PI_TARGET" "
    sudo apt update > /dev/null 2>&1 && 
    sudo apt install -y pulseaudio pulseaudio-utils alsa-utils > /dev/null 2>&1 &&
    sudo usermod -a -G audio \$USER
" || echo -e "${YELLOW}⚠️ Some dependencies may need manual installation${NC}"

echo -e "${GREEN}✅ Dependencies installed${NC}"

# Test the installation
echo -e "${YELLOW}🧪 Testing installation...${NC}"
if ssh "$PI_TARGET" "cd $REMOTE_DIR && ./agent-edge-rs --help" &>/dev/null; then
    echo -e "${GREEN}✅ Binary runs successfully${NC}"
else
    echo -e "${YELLOW}⚠️ Could not test binary execution (may need libraries)${NC}"
fi

echo ""
echo -e "${GREEN}🎉 Deployment completed successfully!${NC}"
echo ""
echo -e "${BLUE}To run on the Pi:${NC}"
echo "  ssh $PI_TARGET"
echo "  cd $REMOTE_DIR"
echo "  ./run-agent.sh"
echo ""
echo -e "${BLUE}To run with logging:${NC}"
echo "  RUST_LOG=info ./run-agent.sh"
echo ""
echo -e "${YELLOW}💡 Note: You may need to logout and login again for audio group permissions to take effect${NC}" 