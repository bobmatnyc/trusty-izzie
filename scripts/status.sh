#!/usr/bin/env bash
# scripts/status.sh — Show status of all trusty-izzie processes.
set -euo pipefail

PID_FILE="/tmp/trusty-daemon.pid"
API_PID_FILE="/tmp/trusty-api.pid"
TG_PID_FILE="/tmp/trusty-telegram.pid"
IPC_SOCKET="/tmp/trusty-izzie.sock"
API_PORT=3456

show_status() {
    local label="$1"
    local pid_file="$2"

    if [[ ! -f "$pid_file" ]]; then
        echo "  $label:  ○ stopped"
        return
    fi

    local pid
    pid=$(cat "$pid_file")

    if kill -0 "$pid" 2>/dev/null; then
        # Get memory usage (RSS in KB on macOS)
        local rss
        rss=$(ps -o rss= -p "$pid" 2>/dev/null | tr -d ' ' || echo "?")
        local rss_mb
        if [[ "$rss" =~ ^[0-9]+$ ]]; then
            rss_mb=$(( rss / 1024 ))
            echo "  $label:  ● running  (PID $pid, ~${rss_mb}MB RSS)"
        else
            echo "  $label:  ● running  (PID $pid)"
        fi
    else
        echo "  $label:  ✗ crashed  (stale PID $pid)"
    fi
}

echo ""
echo "trusty-izzie status"
echo "─────────────────────────────────────"
show_status "daemon   " "$PID_FILE"
show_status "api      " "$API_PID_FILE"
show_status "telegram " "$TG_PID_FILE"

# IPC socket
if [[ -S "$IPC_SOCKET" ]]; then
    echo "  IPC socket:  ● present  ($IPC_SOCKET)"
else
    echo "  IPC socket:  ○ absent"
fi

# API health (non-blocking check)
if command -v curl &>/dev/null; then
    HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" --connect-timeout 1 \
        "http://localhost:$API_PORT/health" 2>/dev/null || echo "000")
    if [[ "$HTTP_CODE" == "200" ]]; then
        echo "  API health:  ● HTTP $HTTP_CODE"
    elif [[ "$HTTP_CODE" == "000" ]]; then
        echo "  API health:  ○ unreachable"
    else
        echo "  API health:  ✗ HTTP $HTTP_CODE"
    fi
fi

# Data directory
DATA_DIR="${TRUSTY_DATA_DIR:-$HOME/.local/share/trusty-izzie}"
DATA_DIR="${DATA_DIR/\~/$HOME}"
if [[ -d "$DATA_DIR" ]]; then
    # Disk usage (suppress errors on permission issues)
    USAGE=$(du -sh "$DATA_DIR" 2>/dev/null | cut -f1 || echo "?")
    echo ""
    echo "  Data dir:    $DATA_DIR  ($USAGE)"
else
    echo ""
    echo "  Data dir:    ○ not found ($DATA_DIR)"
fi
echo ""
