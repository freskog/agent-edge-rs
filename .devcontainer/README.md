# DevContainer Setup for Agent Edge RS

This devcontainer provides a development environment that matches the Raspberry Pi 3 target platform, allowing you to build and test the project directly in Cursor without needing to transfer code to the Pi.

## What's Included

- **Base OS**: Debian Bullseye (64-bit) - matches Raspberry Pi OS Lite
- **Architecture**: ARM64 (aarch64) - matches Pi 3
- **Rust**: Latest stable with aarch64 target
- **Dependencies**: All system libraries needed for building
  - PulseAudio development libraries
  - Build tools (gcc, cmake, pkg-config)
  - TensorFlow Lite dependencies
  - Cross-compilation tools

## Getting Started

1. **Open in DevContainer**: 
   - In Cursor, press `Cmd+Shift+P` (or `Ctrl+Shift+P` on Windows/Linux)
   - Select "Dev Containers: Reopen in Container"
   - Wait for the container to build (first time may take 5-10 minutes)

2. **Build the Project**:
   ```bash
   # Development build
   cargo build
   
   # Release build (optimized)
   cargo build --release
   
   # Run tests
   cargo test
   ```

3. **Cross-compile for Pi 3**:
   ```bash
   # Build for aarch64-unknown-linux-gnu target
   cross build --target aarch64-unknown-linux-gnu --release
   ```

## Environment Details

- **User**: `vscode` (non-root)
- **Working Directory**: `/workspace`
- **Cargo Cache**: Mounted from host for faster builds
- **Rust Target**: `aarch64-unknown-linux-gnu` (Pi 3 compatible)

## Benefits

✅ **Same Environment**: Build in the exact same environment as your Pi 3  
✅ **Faster Development**: No need to transfer code back and forth  
✅ **Full Tooling**: All Rust tools and extensions available  
✅ **Cross-compilation**: Build for Pi 3 from any host platform  
✅ **Dependency Matching**: All system libraries match Pi 3 requirements  

## Limitations

⚠️ **Audio Testing**: Real audio capture won't work in the container  
⚠️ **Hardware Access**: No access to actual microphone hardware  
⚠️ **Performance**: Container performance may differ from actual Pi 3  

## Workflow

1. **Development**: Use the devcontainer for all code changes and builds
2. **Testing**: Run unit tests and integration tests in the container
3. **Deployment**: Copy the built binary to your Pi 3 for final testing
4. **Audio Testing**: Test with real hardware on the Pi 3

## Troubleshooting

### Build Issues
```bash
# Clean and rebuild
cargo clean
cargo build --release

# Check Rust toolchain
rustup show
rustup target list --installed
```

### Container Issues
```bash
# Rebuild container
# In Cursor: Dev Containers: Rebuild Container
```

### Cross-compilation Issues
```bash
# Ensure Docker is running
docker info

# Check cross installation
cross --version
```

## Next Steps

After setting up the devcontainer:

1. Build the project: `cargo build --release`
2. Test the build: `cargo test`
3. Copy binary to Pi 3 for final testing
4. Iterate on development in the container 