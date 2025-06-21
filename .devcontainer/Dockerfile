# Version 1.0.1 - Force rebuild
# Use Ubuntu as base image for better compatibility
FROM ubuntu:22.04

# Prevent interactive prompts during package installation
ENV DEBIAN_FRONTEND=noninteractive
ENV TZ=Etc/UTC

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
    jq \
    htop \
    rsync \
    man-db \
    locales \
    ca-certificates \
    sudo \
    && rm -rf /var/lib/apt/lists/*

# Generate locale (devcontainers expect UTF-8)
RUN locale-gen en_US.UTF-8
ENV LANG=en_US.UTF-8
ENV LANGUAGE=en_US:en
ENV LC_ALL=en_US.UTF-8

# Install Python dependencies
RUN pip3 install numpy

# Install Bazelisk (ARM64 version)
RUN curl -L -o /usr/local/bin/bazelisk https://github.com/bazelbuild/bazelisk/releases/download/v1.19.0/bazelisk-linux-arm64 && \
    chmod +x /usr/local/bin/bazelisk && \
    ln -s /usr/local/bin/bazelisk /usr/local/bin/bazel

# Create marker file to detect container
RUN touch /.dockerenv

# Add a user for VS Code devcontainers
ARG USERNAME=vscode
ARG USER_UID=1000
ARG USER_GID=$USER_UID

RUN groupadd --gid $USER_GID $USERNAME && \
    useradd --uid $USER_UID --gid $USER_GID -m -s /bin/bash $USERNAME && \
    echo "$USERNAME ALL=(ALL) NOPASSWD:ALL" >> /etc/sudoers && \
    # Create /workspace and set ownership to vscode
    mkdir -p /workspace && chown $USERNAME:$USERNAME /workspace && \
    # Copy skeleton files and set ownership
    cp /etc/skel/.bashrc /etc/skel/.profile /home/$USERNAME/ && \
    chown $USERNAME:$USERNAME /home/$USERNAME/.bashrc /home/$USERNAME/.profile

# Make sure bash is default shell for vscode
RUN chsh -s /bin/bash vscode

# Switch to that user for all remaining operations
USER $USERNAME

RUN mkdir -p /home/$USERNAME/agent-edge-rs

WORKDIR /home/$USERNAME/agent-edge-rs

# Configure bash colors and aliases for vscode user
RUN echo 'force_color_prompt=yes' >> ~/.bashrc && \
    echo 'if [ -n "$force_color_prompt" ]; then' >> ~/.bashrc && \
    echo '    PS1='"'"'\${debian_chroot:+(\$debian_chroot)}\[\033[01;32m\]\u@\h\[\033[00m\]:\[\033[01;34m\]\w\[\033[00m\]\$ '"'" >> ~/.bashrc && \
    echo 'fi' >> ~/.bashrc && \
    echo 'alias ls="ls --color=auto"' >> ~/.bashrc && \
    echo 'alias grep="grep --color=auto"' >> ~/.bashrc

# Install Rust and configure environment
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
RUN . $HOME/.cargo/env && rustup target add aarch64-unknown-linux-gnu
RUN echo 'source "$HOME/.cargo/env"' >> ~/.bashrc

# Environment for dev tools
ENV PATH="/home/${USERNAME}/.cargo/bin:${PATH}"
ENV LD_LIBRARY_PATH="/usr/local/lib"
ENV PKG_CONFIG_PATH="/usr/local/lib/pkgconfig"

# Start Bash in login mode so .bashrc is loaded
CMD [ "/bin/bash" ]
