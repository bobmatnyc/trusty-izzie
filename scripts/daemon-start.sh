#!/usr/bin/env bash
# scripts/daemon-start.sh — Start trusty-daemon in the background.
# Usage: daemon-start.sh [release|dev]
set -euo pipefail

PROFILE="${1:-release}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

PID_FILE="/tmp/trusty-daemon.pid"
LOG_FILE="/tmp/trusty-daemon.log"

if [[ "$PROFILE" == "release" ]]; then
    DAEMON_BIN="$PROJECT_DIR/target/release/trusty-daemon"
else
    DAEMON_BIN="$PROJECT_DIR/target/debug/trusty-daemon"
fi

if [[ ! -x "$DAEMON_BIN" ]]; then
    echo "✗ Daemon binary not found: $DAEMON_BIN"
    echo "  Run 'make build' first."
    exit 1
fi

# Check if already running
if [[ -f "$PID_FILE" ]]; then
    OLD_PID=$(cat "$PID_FILE")
    if kill -0 "$OLD_PID" 2>/dev/null; then
        echo "▸ trusty-daemon is already running (PID $OLD_PID)"
        echo "  Use 'make stop' to stop it first."
        exit 0
    else
        echo "▸ Removing stale PID file (PID $OLD_PID no longer running)"
        rm -f "$PID_FILE"
    fi
fi

# Load .env
if [[ -f "$PROJECT_DIR/.env" ]]; then
    set -a
    # shellcheck disable=SC1091
    source "$PROJECT_DIR/.env"
    set +a
fi

echo "▶ Starting trusty-daemon ($PROFILE)…"
echo "  Log → $LOG_FILE"

export RUST_BACKTRACE=1
nohup "$DAEMON_BIN" start --foreground >> "$LOG_FILE" 2>&1 &
DAEMON_PID=$!
echo "$DAEMON_PID" > "$PID_FILE"

# Brief wait to confirm it started
sleep 1
if kill -0 "$DAEMON_PID" 2>/dev/null; then
    echo "✓ trusty-daemon started (PID $DAEMON_PID)"
    echo "  tail -f $LOG_FILE  to follow logs"
    echo "  make stop          to stop"
else
    echo "✗ trusty-daemon failed to start — check $LOG_FILE"
    rm -f "$PID_FILE"
    exit 1
fi
