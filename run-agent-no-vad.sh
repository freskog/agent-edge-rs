#!/bin/bash

echo "🎤 Running Agent WITHOUT VAD"
echo "============================="
echo ""
echo "This runs the original working behavior with no CPU optimization"
echo "Press Ctrl+C to stop"
echo ""

# Disable VAD entirely
export VAD_ENABLED=false

# Run the agent
./target/release/agent-edge-rs 