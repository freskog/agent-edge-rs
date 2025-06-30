#!/bin/bash
set -e

echo "🔧 Setting up Agent Edge RS development environment..."

# Update package lists
echo "📦 Updating package lists..."
sudo apt-get update && sudo apt-get install -y \
    pkg-config \
    libssl-dev \
    libpulse-dev \
    pulseaudio-utils \
    libasound2-dev \
    cmake \
    clang \
    llvm-dev \
    libclang-dev \
    python3 \
    python3-pip \
    curl \
    wget \
    git \
    ca-certificates \
    libudev-dev

# Install Rust target for aarch64 (matching Pi 3)
echo "🦀 Installing Rust aarch64 target..."
rustup target add aarch64-unknown-linux-gnu

# Install cross-compilation tool (optional, for cross-compiling from other platforms)
echo "🔀 Installing cross-compilation tools..."
cargo install cross

# Clean up
echo "🧹 Cleaning up..."
sudo apt-get clean
sudo rm -rf /var/lib/apt/lists/*

echo "✅ Development environment setup complete!"
echo ""
echo "🚀 You can now build the project with:"
echo "   cargo build --release"
echo ""
echo "🧪 Or run tests with:"
echo "   cargo test"
echo ""
echo "📦 To cross-compile for Pi 3:"
echo "   cross build --target aarch64-unknown-linux-gnu --release"
echo ""
echo "🔧 The tflitec crate will automatically download and configure TensorFlow Lite libraries." 