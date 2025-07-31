# Mock Audio Server Testing Guide

The mock audio server provides controlled audio input for testing VAD and wake word detection. You can use it in **two ways**:

1. **📦 Standalone Binary** - Run as separate process (good for manual testing)
2. **🧪 Programmatic Library** - Start from within tests (recommended for automated testing)

## 🧪 **Recommended: Programmatic Testing**

**Best for**: Automated tests, CI/CD, integration tests

```rust
use audio::{MockAudioServer, MockServerConfig};
use std::path::PathBuf;

#[test]
fn test_wake_word_detection() {
    let config = MockServerConfig {
        audio_file: PathBuf::from("../tests/data/hey_mycroft_test.wav"),
        bind_address: "127.0.0.1:0".to_string(), // Random port
        loop_audio: true,
        speed: 4.0, // 4x speed for faster testing
        ..Default::default()
    };

    // Start server automatically, gets random port
    let mock_server = MockAudioServer::new(config)
        .unwrap()
        .start_background()
        .unwrap();

    // Point your services to mock_server.address()
    // Test your pipeline...
    
    // Server stops automatically when dropped
}
```

👉 **See `audio/README_testing.md` and `audio/tests/mock_server_test.rs` for complete examples.**

---

## 📦 **Alternative: Standalone Binary**

The mock audio server allows you to test VAD and wake word detection with controlled audio input using existing wave files instead of relying on live microphone input.

## Features

- **Plays existing WAV files** through the audio protocol (16kHz mono s16le format)
- **Real-time streaming** with proper chunk timing (80ms chunks)
- **Configurable playback speed** for faster testing
- **Looping support** for continuous testing
- **Silence periods** between loops
- **Multiple client support** with proper connection management

## Available Test Files

The following wave files are available in `tests/data/`:

- `hey_mycroft_test.wav` - Wake word recording
- `alexa_test.wav` - Alternative wake word for testing
- `immediate_what_time_is_it.wav` - Quick question after wake word
- `delay_start_what_time_is_it.wav` - Question with delay
- `hesitation_what_time_is_it.wav` - Question with hesitation/pauses

## Quick Start

### 1. Start the Mock Audio Server

```bash
cd audio

# Basic usage - play hey_mycroft_test.wav once
cargo run --bin mock_audio_server

# Specify a different file
cargo run --bin mock_audio_server -- \
  --file ../../tests/data/immediate_what_time_is_it.wav

# Loop continuously for extended testing
cargo run --bin mock_audio_server -- \
  --file ../../tests/data/hey_mycroft_test.wav \
  --loop-audio \
  --silence-duration 3.0

# Faster testing (2x speed)
cargo run --bin mock_audio_server -- \
  --file ../../tests/data/hey_mycroft_test.wav \
  --loop-audio \
  --speed 2.0 \
  --silence-duration 1.0
```

### 2. Test with Wakeword Service

```bash
# In another terminal, start the wakeword service pointing to the mock server
cd wakeword
cargo run -- \
  --server 127.0.0.1:8080 \
  --models hey_mycroft \
  --threshold 0.3 \
  --wakeword-server 127.0.0.1:8081
```

### 3. Test with Full Agent Pipeline

```bash
# Start the agent with the enhanced streaming protocol
cd agent  
cargo run -- --wakeword-address 127.0.0.1:8081
```

## Testing Scenarios

### Wake Word Detection
Test wake word sensitivity with different recordings:

```bash
# Test with standard wake word recording
cargo run --bin mock_audio_server -- \
  --file ../../tests/data/hey_mycroft_test.wav \
  --loop-audio \
  --silence-duration 4.0

# Test with alternative wake word (for false positive testing)
cargo run --bin mock_audio_server -- \
  --file ../../tests/data/alexa_test.wav \
  --loop-audio \
  --silence-duration 4.0
```

### VAD Boundary Detection
Test how well the VAD detects speech start/end with different speech patterns:

```bash
# Test with immediate speech after wake word
cargo run --bin mock_audio_server -- \
  --file ../../tests/data/immediate_what_time_is_it.wav \
  --loop-audio \
  --silence-duration 5.0

# Test with delayed speech (should trigger VAD timeout)
cargo run --bin mock_audio_server -- \
  --file ../../tests/data/delay_start_what_time_is_it.wav \
  --loop-audio \
  --silence-duration 6.0

# Test with hesitation patterns
cargo run --bin mock_audio_server -- \
  --file ../../tests/data/hesitation_what_time_is_it.wav \
  --loop-audio \
  --silence-duration 5.0
```

Watch the logs for VAD events:
- `🗣️ VAD: Speech started` 
- `🔇 VAD: Speech ended`

### End-to-End Agent Testing
Test the complete pipeline:

1. **Start mock server** with wake word + speech pattern
2. **Start wakeword service** with event server
3. **Start agent** with streaming protocol
4. **Watch logs** for the complete flow:
   - Wake word detection
   - Utterance session start
   - Audio chunk streaming
   - VAD end-of-speech detection
   - STT transcription
   - LLM processing
   - TTS response

### Performance Testing
Test system performance and timing:

```bash
# Fast iteration testing (4x speed)
cargo run --bin mock_audio_server -- \
  --file ../../tests/data/hey_mycroft_test.wav \
  --loop-audio \
  --speed 4.0 \
  --silence-duration 0.5

# Stress testing with continuous playback
cargo run --bin mock_audio_server -- \
  --file ../../tests/data/immediate_what_time_is_it.wav \
  --loop-audio \
  --silence-duration 0.1
```

## Command Line Options

```bash
cargo run --bin mock_audio_server -- --help
```

### Key Options:
- `--file` - WAV file to play (default: `../../tests/data/hey_mycroft_test.wav`)
- `--address` - Server bind address (default: `127.0.0.1:8080`)
- `--loop-audio` - Loop the file continuously
- `--silence-duration` - Seconds of silence after file ends before looping (default: 2.0)
- `--speed` - Playback speed multiplier (default: 1.0, real-time)

## Audio File Requirements

The mock server expects audio files in specific format:
- **Sample Rate:** 16kHz
- **Channels:** Mono (1 channel)
- **Bit Depth:** 16-bit signed integer (s16le)
- **Format:** WAV

All files in `tests/data/` are already in the correct format.

### Converting External Files
If you have audio files in other formats:

```bash
# Convert to proper format using ffmpeg
ffmpeg -i your_audio.mp3 -ar 16000 -ac 1 -sample_fmt s16 your_audio.wav

# Use with mock server
cargo run --bin mock_audio_server -- --file your_audio.wav
```

## Debugging Tips

### Connection Issues
- Verify the mock server binds to the expected address
- Check that wakeword service connects to mock server, not real audio service
- Monitor connection logs: `📡 Client X subscribed to audio`

### Audio Format Issues
The server will validate the audio file format on startup:
```
📊 Audio file info: 16000Hz, 1 channels, 16 bits
```

If you see format errors, the file needs to be converted.

### VAD Tuning
- Monitor VAD state transitions in wakeword service logs
- Test with different speech patterns (immediate vs delayed vs hesitation)
- Adjust VAD thresholds in `wakeword/src/vad.rs`

### Wake Word Sensitivity
- Adjust `--threshold` parameter on wakeword service (lower = more sensitive)
- Test with different wake word recordings
- Use `alexa_test.wav` to test false positive rejection

### Performance Monitoring
- Use `--speed` parameter for faster iteration during development
- Monitor chunk streaming logs: `🎵 Sent X audio chunks`
- Watch for client disconnect messages

## Example Test Session

```bash
# Terminal 1: Start mock server with wake word + question
cd audio
cargo run --bin mock_audio_server -- \
  --file ../../tests/data/immediate_what_time_is_it.wav \
  --loop-audio \
  --silence-duration 4.0

# Terminal 2: Start wakeword service with enhanced protocol
cd wakeword
cargo run -- \
  --server 127.0.0.1:8080 \
  --models hey_mycroft \
  --threshold 0.4 \
  --wakeword-server 127.0.0.1:8081

# Terminal 3: Start agent with streaming protocol  
cd agent
cargo run -- --wakeword-address 127.0.0.1:8081

# Expected log flow:
# 🎵 Mock server: streaming audio chunks
# 🎯 Wakeword service: "hey mycroft" detected
# 🎤 Agent: utterance session started  
# 🎵 Agent: receiving audio chunks
# 🔇 Agent: VAD detected end of speech
# 📝 Agent: STT transcription "what time is it"
# 🧠 Agent: LLM processing
# 🔊 Agent: TTS response
```

## Advanced Testing

### Multiple File Testing
To test with different files sequentially, restart the mock server with different `--file` parameters, or use a script:

```bash
#!/bin/bash
files=(
    "../../tests/data/hey_mycroft_test.wav"
    "../../tests/data/immediate_what_time_is_it.wav"
    "../../tests/data/hesitation_what_time_is_it.wav"
)

for file in "${files[@]}"; do
    echo "Testing with: $file"
    cargo run --bin mock_audio_server -- \
        --file "$file" \
        --silence-duration 3.0 &
    
    sleep 10  # Let it run for 10 seconds
    kill $!   # Stop the server
    sleep 2   # Brief pause between tests
done
```

### Integration with CI/CD
The mock server can be used in automated testing:

```bash
# Start mock server in background
cargo run --bin mock_audio_server -- \
  --file ../../tests/data/hey_mycroft_test.wav \
  --address 127.0.0.1:8080 &
MOCK_PID=$!

# Run your tests
cargo test --test integration_tests

# Clean up
kill $MOCK_PID
```

This setup provides controlled, repeatable testing of the complete VAD + wake word + agent pipeline using real audio recordings! 