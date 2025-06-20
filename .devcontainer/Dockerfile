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

# Create vscode user with proper permissions
ARG USERNAME=vscode
ARG USER_UID=1000
ARG USER_GID=$USER_UID

RUN groupadd --gid $USER_GID $USERNAME && \
    useradd --uid $USER_UID --gid $USER_GID -m $USERNAME && \
    echo "$USERNAME ALL=(ALL) NOPASSWD:ALL" >> /etc/sudoers

# Set environment variables for development
ENV LD_LIBRARY_PATH="/usr/local/lib"
ENV PKG_CONFIG_PATH="/usr/local/lib/pkgconfig"

# Switch to user by default
USER $USERNAME

# Set up Rust for the vscode user
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH="/home/$USERNAME/.cargo/bin:${PATH}"

# Configure shell environment
RUN echo 'export PATH="/home/vscode/.cargo/bin:$PATH"' >> /home/$USERNAME/.bashrc && \
    echo 'export PS1="\[\033[01;32m\]\u@\h\[\033[00m\]:\[\033[01;34m\]\w\[\033[00m\]\$ "' >> /home/$USERNAME/.bashrc && \
    echo 'set -o vi' >> /home/$USERNAME/.bashrc

# Set default shell to bash
SHELL ["/bin/bash", "-c"]

CMD ["/bin/bash"]
