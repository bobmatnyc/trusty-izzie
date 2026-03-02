#!/usr/bin/env bash
# scripts/daemon-stop.sh — Stop trusty-daemon (and API if running).
set -euo pipefail

PID_FILE="/tmp/trusty-daemon.pid"
API_PID_FILE="/tmp/trusty-api.pid"
TG_PID_FILE="/tmp/trusty-telegram.pid"

stop_process() {
    local label="$1"
    local pid_file="$2"

    if [[ ! -f "$pid_file" ]]; then
        echo "▸ $label is not running (no PID file)"
        return 0
    fi

    local pid
    pid=$(cat "$pid_file")

    if ! kill -0 "$pid" 2>/dev/null; then
        echo "▸ $label PID $pid is no longer running (removing stale PID file)"
        rm -f "$pid_file"
        return 0
    fi

    echo "▶ Stopping $label (PID $pid)…"
    kill -TERM "$pid"

    # Wait up to 5 seconds for graceful shutdown
    for i in $(seq 1 10); do
        if ! kill -0 "$pid" 2>/dev/null; then
            break
        fi
        sleep 0.5
    done

    if kill -0 "$pid" 2>/dev/null; then
        echo "  SIGTERM timed out — sending SIGKILL"
        kill -KILL "$pid"
    fi

    rm -f "$pid_file"
    echo "✓ $label stopped"
}

stop_process "trusty-daemon"  "$PID_FILE"
stop_process "trusty-api"     "$API_PID_FILE"
stop_process "trusty-telegram" "$TG_PID_FILE"
