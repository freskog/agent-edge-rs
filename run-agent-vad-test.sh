#!/bin/bash

echo "🎤 Testing WebRTC VAD Integration"
echo "=================================="
echo ""
echo "This will run the agent with debug logging to show VAD decisions"
echo "Press Ctrl+C to stop"
echo ""

# Enable VAD 
export VAD_ENABLED=true

# Run the agent
./target/release/agent-edge-rs 