#!/bin/bash

echo "🎤 Testing Corrected WebRTC VAD"
echo "==============================="
echo ""
echo "This uses the corrected audio pipeline:"
echo "  - S16LE capture (WebRTC VAD native format)"
echo "  - 6-channel → channel 0 extraction"
echo "  - 16kHz, 16-bit audio"
echo "  - No energy pre-filtering"
echo ""
echo "Press Ctrl+C to stop"
echo ""

# Enable VAD with corrected pipeline
export VAD_ENABLED=true

# Run the agent
./target/release/agent-edge-rs 