# Dual-Channel Audio with Agent-Controlled Spotify Ducking

This document describes the enhanced audio system that uses the ReSpeaker XVF3800's dual channels for optimal wakeword detection and STT processing, with intelligent Spotify ducking controlled by the agent.

## Overview

### The Problem
The ReSpeaker XVF3800 has two channels with different characteristics:
- **Channel 0**: Aggressively tuned for AEC, great for STT but can suppress speech if reference audio is louder
- **Channel 1**: Better for wakeword detection, avoids speech suppression but still reduces noise

### The Solution
- Use **Channel 1** for wakeword detection (avoids false negatives)
- Use **Channel 0** for STT streaming (cleanest audio for speech recognition)
- Duck Spotify when wakeword detected, let agent decide when to resume

## Architecture

### Components

1. **DualChannelAudioManager** (`src/dual_channel_audio.rs`)
   - Manages two `AudioCapture` instances (one per channel)
   - Handles Spotify ducking via `playerctl`
   - Tracks pause state and safety timeouts

2. **Enhanced Protocol** (`src/protocol.rs`)
   - `WakewordDetected` now includes `spotify_was_paused: bool`
   - New producer messages: `UnpauseSpotify`, `SpotifyPaused`, `SpotifyResumed`

3. **Spotify Controller** (`src/spotify_controller.rs`)
   - Uses `playerctl` for robust media control
   - Tracks whether WE paused the music (vs user/other app)
   - Supports specific player selection (e.g., "spotifyd")

## Message Flow

### 1. Wakeword Detection Flow
```
1. Audio from Channel 1 → Wakeword Detection
2. Wakeword detected → Spotify paused (if playing)
3. WakewordDetected message sent with spotify_was_paused=true/false
4. Agent receives wakeword + Spotify state info
```

### 2. STT Processing Flow
```
1. Audio from Channel 0 → STT streaming to agent
2. Agent processes speech with clean audio
3. Agent generates response
```

### 3. Agent-Controlled Resume Flow
```
1. Agent finishes processing → Uses Spotify Web API to resume/control music
2. Agent has full control over music state via Web API
3. Clean separation: Audio service only pauses, Agent handles all resume logic
```

## Protocol Messages

### Consumer Messages (Port 8080)
```rust
WakewordDetected {
    model: String,
    timestamp: u64,
    spotify_was_paused: bool,  // NEW: Whether Spotify was paused
}
```

### Producer Messages (Port 8081)
```rust
// No additional messages needed!
// Agent uses Spotify Web API for all resume/control operations
// Audio service only provides pause state information via WakewordDetected
```

## Agent Integration Examples

### Python Agent Example
```python
class EnhancedAudioAgent:
    def __init__(self):
        self.consumer_conn = connect_to_consumer_port()  # Port 8080
        self.producer_conn = connect_to_producer_port()  # Port 8081
        self.spotify_paused_by_us = False
        self.music_changed_during_processing = False
        
    def handle_wakeword(self, message):
        """Handle wakeword detection with Spotify state"""
        print(f"Wakeword: {message.model}")
        self.spotify_paused_by_us = message.spotify_was_paused
        
        if self.spotify_paused_by_us:
            print("🔇 Spotify was paused for this wakeword")
        else:
            print("🎵 No music was playing")
    
    def process_user_command(self, command):
        """Process user command and track music changes"""
        if "play" in command or "music" in command:
            # Agent changed music via Spotify Web API
            self.music_changed_during_processing = True
            self.handle_music_command(command)
        else:
            # Regular command processing
            self.handle_regular_command(command)
    
    def finish_response(self):
        """Called when agent is done responding"""
        if self.spotify_paused_by_us:
            if self.music_changed_during_processing:
                print("🎵 Music changed - already handled via Web API")
            else:
                print("🔊 Resuming music via Spotify Web API")
                self.spotify_web_api.resume_playback()
        
        # Reset state for next interaction
        self.spotify_paused_by_us = False
        self.music_changed_during_processing = False
```

### Use Cases Handled

#### Case 1: Simple Query (No Music Change)
```
1. User: "Hey Jarvis, what's the weather?"
2. Wakeword detected → Spotify paused → spotify_was_paused=true
3. Agent responds with weather
4. Agent resumes via Web API → Music continues
```

#### Case 2: Music Command
```
1. User: "Hey Jarvis, play some jazz"
2. Wakeword detected → Spotify paused → spotify_was_paused=true  
3. Agent changes music via Spotify Web API (new music already playing)
4. No additional resume needed
```

#### Case 3: No Music Playing
```
1. User: "Hey Jarvis, set a timer"
2. Wakeword detected → No music to pause → spotify_was_paused=false
3. Agent responds
4. No resume needed (nothing was playing)
```

#### Case 4: Long Conversation
```
1. User: "Hey Jarvis, read me the news"
2. Wakeword detected → Spotify paused → spotify_was_paused=true
3. Agent reads news for 5 minutes
4. Agent resumes via Web API when done → Music continues
```

## CLI Usage

### Basic Dual-Channel Mode
```bash
./audio_service --dual-channel-mode --input-device "ReSpeaker XVF3800"
```

### With Specific Spotify Player
```bash
./audio_service --dual-channel-mode --spotify-player "spotifyd"
```

### With Safety Timeout
```bash
./audio_service --dual-channel-mode --auto-unduck-timeout 180  # 3 minutes
```

## Installation Requirements

### On Raspberry Pi
```bash
# Install playerctl
sudo apt update
sudo apt install playerctl

# Ensure spotifyd has MPRIS support
# In spotifyd config:
use_mpris = true
```

## Benefits

### Audio Quality
- **Optimal Wakeword Detection**: Channel 1 avoids speech suppression
- **Clean STT Audio**: Channel 0 provides post-AEC audio for recognition
- **Synchronized Channels**: Both from same device, perfect timing

### User Experience  
- **Smart Ducking**: Only pauses when music is actually playing
- **Agent Control**: Handles conversations, reading, music changes intelligently
- **Robust Recovery**: Safety timeout prevents infinite pausing
- **State Awareness**: Agent knows whether to use UnpauseSpotify or Web API

### Technical Robustness
- **playerctl**: More reliable than direct D-Bus
- **State Tracking**: Prevents resuming music that wasn't playing
- **Error Handling**: Graceful fallbacks and comprehensive logging
- **Backwards Compatible**: Existing agents continue to work

## Future Enhancements

1. **Volume Ducking**: Instead of pause/resume, lower volume during speech
2. **Multiple Players**: Support multiple simultaneous media players
3. **Smart Timing**: Pause only after speech starts (reduce false positives)
4. **Integration**: Direct integration with music streaming APIs

This system provides the optimal balance of audio quality, user experience, and technical robustness for voice assistant applications with the ReSpeaker XVF3800.
