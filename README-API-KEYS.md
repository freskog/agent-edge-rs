# API Keys Configuration

This project uses external cloud services that require API keys for authentication.

## Required API Keys

1. **Fireworks AI** - For streaming speech-to-text (STT)
   - Sign up at: https://fireworks.ai/
   - Get your API key from: https://fireworks.ai/api-keys
   - Format: Starts with `fw_`

2. **Groq** - For LLM inference
   - Sign up at: https://groq.com/
   - Get your API key from: https://console.groq.com/keys
   - Format: Starts with `gsk_`

3. **ElevenLabs** - For text-to-speech (TTS)  
   - Sign up at: https://elevenlabs.io/
   - Get your API key from: https://elevenlabs.io/app/settings/api-keys
   - Format: Alphanumeric string (typically 10+ characters)

## Setup Instructions

### Development Setup

1. Copy the example environment file:
   ```bash
   cp .env.example .env
   ```

2. Edit `.env` and add your real API keys:
   ```bash
   FIREWORKS_API_KEY=fw_your_actual_fireworks_key_here
   GROQ_API_KEY=gsk_your_actual_groq_key_here
   ELEVENLABS_API_KEY=your_actual_elevenlabs_key_here
   ```

3. The `.env` file is already in `.gitignore` so your keys won't be committed.

### Production Setup

Set environment variables directly:
```bash
export FIREWORKS_API_KEY=fw_your_key_here
export GROQ_API_KEY=gsk_your_key_here
export ELEVENLABS_API_KEY=your_key_here
./agent-edge-rs
```

## Security Features

- API keys are wrapped in `SecretBox` to prevent accidental logging
- Keys are validated at startup with helpful error messages
- Keys are never displayed in debug output (shows "..." instead)
- Memory is securely wiped when the program exits

## Testing Configuration

Run this to test your configuration:
```bash
cargo test config::
```

The tests will validate:
- Key format validation works correctly
- Missing environment variables are handled properly
- Keys are properly secured in memory
