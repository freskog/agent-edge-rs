#!/usr/bin/env bash
set -euo pipefail

PI_HOST="${PI_HOST:-freskog@mycroft.local}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
RELEASE_DIR="$PROJECT_ROOT/target/release"

# Map deploy names to cargo binary names and systemd unit names
declare -A BIN_MAP=(
    [audio]=audio
    [led_controller]=led_controller
)
declare -A UNIT_MAP=(
    [audio]=audio
    [led_controller]=led-controller
)

SERVICES=(audio led_controller)

usage() {
    cat <<EOF
Usage: $(basename "$0") <command> [service]

Commands:
  build [service|all]      Build binary (default: all)
  deploy [service|all]     Build + stop + copy + start (default: all)
  push [service|all]       Stop + copy + start without building
  services                 Copy systemd service files to Pi and reload
  status                   Show status of all services on Pi
  logs [service]           Tail logs (all services or specific one)
  restart [service|all]    Restart service(s) on Pi
  stop [service|all]       Stop service(s) on Pi
  start [service|all]      Start service(s) on Pi

Services: ${SERVICES[*]}

Environment:
  PI_HOST   SSH target (default: freskog@mycroft.local)
EOF
    exit 1
}

build_service() {
    local svc="$1"
    local bin="${BIN_MAP[$svc]}"
    echo "==> Building $bin..."
    cargo build --release --bin "$bin" --manifest-path "$PROJECT_ROOT/Cargo.toml"
    echo "    Built: $RELEASE_DIR/$bin"
}

stop_service() {
    local svc="$1"
    local unit="${UNIT_MAP[$svc]}"
    echo "==> Stopping $svc on Pi..."
    ssh "$PI_HOST" "systemctl --user stop $unit.service" 2>/dev/null || true
}

start_service() {
    local svc="$1"
    local unit="${UNIT_MAP[$svc]}"
    echo "==> Starting $svc on Pi..."
    ssh "$PI_HOST" "systemctl --user start $unit.service"
}

copy_binary() {
    local svc="$1"
    local bin="${BIN_MAP[$svc]}"
    echo "==> Copying $bin to Pi..."
    scp "$RELEASE_DIR/$bin" "$PI_HOST:~/$bin"
}

show_status() {
    local svc="$1"
    local unit="${UNIT_MAP[$svc]:-$svc}"
    ssh "$PI_HOST" "systemctl --user status $unit.service --no-pager" 2>/dev/null || true
}

resolve_targets() {
    local target="${1:-all}"
    if [[ "$target" == "all" ]]; then
        echo "${SERVICES[@]}"
    else
        if [[ -z "${BIN_MAP[$target]+x}" ]]; then
            echo "Unknown service: $target" >&2
            echo "Available: ${SERVICES[*]}" >&2
            exit 1
        fi
        echo "$target"
    fi
}

cmd="${1:-}"
shift || true

case "$cmd" in
    build)
        for svc in $(resolve_targets "${1:-all}"); do
            build_service "$svc"
        done
        ;;
    deploy)
        for svc in $(resolve_targets "${1:-all}"); do
            build_service "$svc"
            stop_service "$svc"
            copy_binary "$svc"
            start_service "$svc"
            echo "==> Status of $svc:"
            show_status "$svc"
            echo
        done
        ;;
    push)
        for svc in $(resolve_targets "${1:-all}"); do
            stop_service "$svc"
            copy_binary "$svc"
            start_service "$svc"
            echo "==> Status of $svc:"
            show_status "$svc"
            echo
        done
        ;;
    services)
        echo "==> Copying service files to Pi..."
        ssh "$PI_HOST" "mkdir -p ~/.config/systemd/user"
        scp "$SCRIPT_DIR"/systemd/*.service "$PI_HOST:~/.config/systemd/user/"
        echo "==> Reloading systemd..."
        ssh "$PI_HOST" "systemctl --user daemon-reload"
        echo "==> Enabling all services..."
        ssh "$PI_HOST" "systemctl --user enable spotifyd mpv led-controller audio agent-edge"
        echo "==> Enabling linger (start services at boot without login)..."
        ssh "$PI_HOST" "sudo loginctl enable-linger freskog"
        echo "Done. Services will start on next reboot, or run: ./deploy.sh start all"
        ;;
    status)
        ALL_SERVICES="spotifyd mpv led-controller audio agent-edge"
        target="${1:-all}"
        if [[ "$target" == "all" ]]; then
            services_list="$ALL_SERVICES"
        else
            services_list="$target"
        fi
        for svc in $services_list; do
            echo "--- $svc ---"
            show_status "$svc"
            echo
        done
        ;;
    logs)
        target="${1:-}"
        if [[ -n "$target" ]]; then
            ssh "$PI_HOST" "journalctl --user -u $target.service -f --no-pager"
        else
            ssh "$PI_HOST" "journalctl --user -u spotifyd -u mpv -u led-controller -u audio -u agent-edge -f --no-pager"
        fi
        ;;
    restart)
        ALL_SERVICES="spotifyd mpv led-controller audio agent-edge"
        target="${1:-all}"
        if [[ "$target" == "all" ]]; then
            services_list="$ALL_SERVICES"
        else
            services_list="$target"
        fi
        for svc in $services_list; do
            echo "==> Restarting $svc..."
            ssh "$PI_HOST" "systemctl --user restart $svc.service"
        done
        echo "==> Status:"
        for svc in $services_list; do
            show_status "$svc"
            echo
        done
        ;;
    stop)
        for svc in $(resolve_targets "${1:-all}"); do
            stop_service "$svc"
        done
        ;;
    start)
        for svc in $(resolve_targets "${1:-all}"); do
            start_service "$svc"
        done
        ;;
    *)
        usage
        ;;
esac
