#!/bin/bash
# Set library path and run the agent
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LD_LIBRARY_PATH="$DIR/lib:$LD_LIBRARY_PATH" exec "$DIR/agent-edge" "$@"
