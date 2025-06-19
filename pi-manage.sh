#!/bin/bash
set -e

# Configuration
PI_HOST="freskog@10.10.100.98"

# Function to show usage
show_usage() {
    echo "Usage: $0 <command>"
    echo ""
    echo "Commands:"
    echo "  test      - Test if agent runs"
    echo "  start     - Start agent in background"
    echo "  stop      - Stop running agent"
    echo "  status    - Check if agent is running"
    echo "  logs      - Show agent logs (last 20 lines)"
    echo "  follow    - Follow agent logs in real-time"
    echo "  shell     - Open SSH shell to Pi"
    echo "  info      - Show Pi system information"
    echo ""
}

# Check if command provided
if [ $# -eq 0 ]; then
    show_usage
    exit 1
fi

COMMAND=$1

case $COMMAND in
    "test")
        echo "🧪 Testing agent on Pi..."
        ssh "$PI_HOST" << 'EOSSH'
            cd ~/agent-edge
            ./run-agent.sh --help
EOSSH
        ;;
    
    "start")
        echo "🚀 Starting agent on Pi..."
        ssh "$PI_HOST" << 'EOSSH'
            cd ~/agent-edge
            # Kill any existing agent
            pkill -f "./agent-edge" || true
            # Start in background with logs
            nohup ./run-agent.sh > agent.log 2>&1 &
            echo "Agent started with PID: $!"
            echo "Use 'pi-manage.sh logs' to see output"
EOSSH
        ;;
    
    "stop")
        echo "🛑 Stopping agent on Pi..."
        ssh "$PI_HOST" << 'EOSSH'
            if pkill -f "./agent-edge"; then
                echo "Agent stopped"
            else
                echo "No agent process found"
            fi
EOSSH
        ;;
    
    "status")
        echo "📊 Checking agent status..."
        ssh "$PI_HOST" << 'EOSSH'
            if pgrep -f "./agent-edge" > /dev/null; then
                echo "✅ Agent is running"
                echo "Process info:"
                ps aux | grep "./agent-edge" | grep -v grep
            else
                echo "❌ Agent is not running"
            fi
EOSSH
        ;;
    
    "logs")
        echo "📋 Showing agent logs..."
        ssh "$PI_HOST" << 'EOSSH'
            cd ~/agent-edge
            if [ -f agent.log ]; then
                echo "Last 20 lines of agent.log:"
                tail -20 agent.log
            else
                echo "No log file found"
            fi
EOSSH
        ;;
    
    "follow")
        echo "📋 Following agent logs (Ctrl+C to stop)..."
        ssh "$PI_HOST" << 'EOSSH'
            cd ~/agent-edge
            if [ -f agent.log ]; then
                tail -f agent.log
            else
                echo "No log file found"
            fi
EOSSH
        ;;
    
    "shell")
        echo "🐚 Opening SSH shell to Pi..."
        ssh "$PI_HOST"
        ;;
    
    "info")
        echo "📋 Pi system information..."
        ssh "$PI_HOST" << 'EOSSH'
            echo "=== System Info ==="
            cat /etc/os-release | grep PRETTY_NAME
            echo "Architecture: $(uname -m)"
            echo "Kernel: $(uname -r)"
            echo "Uptime: $(uptime)"
            echo ""
            echo "=== Memory ==="
            free -h
            echo ""
            echo "=== Disk ==="
            df -h / | tail -1
            echo ""
            echo "=== Temperature ==="
            if [ -f /sys/class/thermal/thermal_zone0/temp ]; then
                temp=$(cat /sys/class/thermal/thermal_zone0/temp)
                echo "CPU: $((temp/1000))°C"
            else
                echo "Temperature sensor not available"
            fi
            echo ""
            echo "=== Network ==="
            ip route | grep default
EOSSH
        ;;
    
    *)
        echo "❌ Unknown command: $COMMAND"
        show_usage
        exit 1
        ;;
esac 