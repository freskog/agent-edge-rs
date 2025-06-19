# Deployment Guide

This project uses TensorFlow Lite C bindings that require both the binary and shared libraries to be deployed together. Due to the Docker container networking setup, we use a two-step deployment process.

## Two-Step Deployment Process

### Step 1: Build Inside Container

Run this command **inside the Docker container** (where you have the Rust toolchain):

```bash
./build-for-pi.sh
```

This will:
- Cross-compile the binary for `aarch64-unknown-linux-gnu` (64-bit Raspberry Pi)
- Find and bundle the required `libtensorflowlite_c.so` library
- Create a deployment package in the `deploy-pi/` directory
- Generate a wrapper script that sets the correct library path

### Step 2: Deploy From Host

Run this command **from your Mac terminal** (outside the container, where you have network access to the Pi):

```bash
./deploy-from-host.sh
```

This will:
- Transfer the deployment package to the Raspberry Pi
- Test the deployment
- Verify that the binary runs correctly

## Alternative: Single Script (Container Only)

If your container has network access to the Pi, you can use:

```bash
./deploy-pi.sh
```

## Pi Management

Once deployed, manage the agent remotely:

```bash
./pi-manage.sh <command>
```

Available commands:
- `start` - Start the agent
- `stop` - Stop the agent  
- `status` - Check if running
- `logs` - Show recent logs
- `follow` - Follow live logs
- `info` - Show system info
- `shell` - Open SSH shell

## File Structure

After deployment, the Pi will have:

```
~/agent-edge/
├── agent-edge                 # Main binary
├── lib/
│   └── libtensorflowlite_c.so # TensorFlow Lite library
└── run-agent.sh              # Wrapper script
```

## Requirements

### Development Environment (Container)
- Rust toolchain with `aarch64-unknown-linux-gnu` target
- TensorFlow Lite C bindings (`tflitec` crate)

### Target Environment (Raspberry Pi)
- 64-bit Raspberry Pi OS (glibc-based)
- SSH access configured
- Standard C runtime libraries

### Host Environment (Mac)
- SSH access to Pi
- `rsync` command available

## SSH Setup

Ensure SSH keys are set up for passwordless access:

```bash
ssh-copy-id <user>@<raspberry.pi.ip>
```

## Troubleshooting

### "libtensorflowlite_c.so: cannot open shared object file"
- The library wasn't bundled correctly
- Rebuild with `./build-for-pi.sh`
- Use the wrapper script `./run-agent.sh` instead of running the binary directly

### "SSH connection failed"
- From container: Expected, use the two-step process
- From host: Check network connectivity and SSH keys

### "Binary not found"
- Run `./build-for-pi.sh` first to create the deployment package

### Cross-compilation issues
- Ensure `aarch64-unknown-linux-gnu` target is installed
- Check that the build completes without errors

## Configuration

To change the Pi IP or username, edit the scripts:
- `deploy-pi.sh`
- `