# Systemd User Services for agent-edge-rs

This directory contains systemd user service files for running the audio API and wakeword detection services automatically.

## Why User Services?

Since we're using PipeWire for audio (which runs in the user session), these services need to run as user services rather than system services. User services have access to the user's audio session and PipeWire instance.

## Services

### audio-api.service
- Runs the main audio API server that handles audio capture and playback
- Binds to `0.0.0.0:50051` by default
- Depends on PipeWire being available
- Automatically restarts on failure

### wakeword.service  
- Runs the wake word detection client
- Connects to the audio API server
- Automatically restarts every 5 seconds if it loses connection
- Depends on audio-api.service being available

## Installation

1. **Build the binaries first:**
   ```bash
   # Build audio API
   cd audio
   cargo build --release
   cp target/release/audio_api ~/.local/bin/
   
   # Build wakeword detection  
   cd ../wakeword
   cargo build --release
   cp target/release/wakeword ~/.local/bin/
   ```

2. **Install the services:**
   ```bash
   cd systemd
   ./install.sh
   ```

3. **Enable and start the services:**
   ```bash
   # Enable services to start automatically
   systemctl --user enable audio-api.service
   systemctl --user enable wakeword.service
   
   # Start the services now
   systemctl --user start audio-api.service
   systemctl --user start wakeword.service
   ```

4. **Enable lingering (optional but recommended):**
   ```bash
   # This keeps services running even after you log out
   sudo loginctl enable-linger $USER
   ```

## Usage

### Check Status
```bash
systemctl --user status audio-api.service
systemctl --user status wakeword.service
```

### View Logs
```bash
# Live logs
journalctl --user -f -u audio-api.service
journalctl --user -f -u wakeword.service

# All logs
journalctl --user -u audio-api.service
journalctl --user -u wakeword.service
```

### Control Services
```bash
# Stop services
systemctl --user stop wakeword.service
systemctl --user stop audio-api.service

# Restart services
systemctl --user restart audio-api.service
systemctl --user restart wakeword.service

# Disable automatic startup
systemctl --user disable wakeword.service
systemctl --user disable audio-api.service
```

## Benefits of This Approach

1. **Automatic Recovery**: If the audio process crashes or is restarted, wakeword automatically reconnects
2. **Clean Separation**: Each service has a single responsibility
3. **Standard Tooling**: Use familiar systemd commands for management
4. **User Session Integration**: Works properly with PipeWire and user audio
5. **Resource Management**: Built-in memory limits and restart policies
6. **Logging**: Integrated with systemd journal

## Troubleshooting

### Audio API won't start
- Check if PipeWire is running: `systemctl --user status pipewire.service`
- List audio devices: `~/.local/bin/audio_api --list-devices`
- Check the logs: `journalctl --user -u audio-api.service`

### Wakeword keeps restarting
- This is normal if audio-api isn't running
- Check audio-api status first: `systemctl --user status audio-api.service`
- The 5-second restart delay prevents excessive resource usage

### Services don't start after reboot
- Enable lingering: `sudo loginctl enable-linger $USER`
- Check if services are enabled: `systemctl --user is-enabled audio-api.service wakeword.service`

## Configuration

You can modify the service files in `~/.config/systemd/user/` to customize:
- Audio device selection
- Detection thresholds
- Memory limits
- Restart policies
- Logging levels

After making changes, run:
```bash
systemctl --user daemon-reload
systemctl --user restart audio-api.service wakeword.service
``` 