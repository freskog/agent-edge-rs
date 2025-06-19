# Use Ubuntu as base image for better compatibility
FROM ubuntu:22.04

# Prevent interactive prompts during package installation
ENV DEBIAN_FRONTEND=noninteractive

# Install system dependencies
RUN apt-get update && apt-get install -y \
    curl \
    wget \
    build-essential \
    gcc \
    g++ \
    pkg-config \
    libssl-dev \
    git \
    # Audio system dependencies
    libasound2-dev \
    libpulse-dev \
    # Python dependencies for tflitec
    python3 \
    python3-pip \
    python3-dev \
    # Additional dependencies
    libclang-dev \
    tar \
    gzip \
    ca-certificates \
    # Clean up
    && rm -rf /var/lib/apt/lists/*

# Install Python dependencies
RUN pip3 install numpy

# Install Bazelisk (ARM64 version for Apple Silicon and Pi)
RUN curl -L -o /usr/local/bin/bazelisk https://github.com/bazelbuild/bazelisk/releases/download/v1.19.0/bazelisk-linux-arm64 && \
    chmod +x /usr/local/bin/bazelisk && \
    ln -s /usr/local/bin/bazelisk /usr/local/bin/bazel

# Install Rust
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH="/root/.cargo/bin:${PATH}"

# Add Rust target for ARM64 (Raspberry Pi 3)
RUN rustup target add aarch64-unknown-linux-gnu

# Set working directory
WORKDIR /workspace

# Create marker file to indicate we're in a container
RUN touch /.dockerenv

# Set environment variables for development
ENV LD_LIBRARY_PATH="/usr/local/lib"
ENV PKG_CONFIG_PATH="/usr/local/lib/pkgconfig"

CMD ["/bin/bash"] 