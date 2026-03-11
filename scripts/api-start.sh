#!/usr/bin/env bash
# scripts/api-start.sh - Start the trusty-api REST server in the background.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

API_BIN="$PROJECT_DIR/target/release/trusty-api"
PID_FILE="/tmp/trusty-api.pid"
LOG_FILE="/tmp/trusty-api.log"
PORT=3456

if [[ ! -x "$API_BIN" ]]; then
    echo "✗ API binary not found: $API_BIN"
    echo "  Run 'make build' first."
    exit 1
fi

if [[ -f "$PID_FILE" ]]; then
    OLD_PID=$(cat "$PID_FILE")
    if kill -0 "$OLD_PID" 2>/dev/null; then
        echo "▸ trusty-api is already running (PID $OLD_PID, port $PORT)"
        exit 0
    fi
    rm -f "$PID_FILE"
fi

if [[ -f "$PROJECT_DIR/.env" ]]; then
    set -a
    # shellcheck disable=SC1091
    source "$PROJECT_DIR/.env"
    set +a
fi

echo "▶ Starting trusty-api on port $PORT…"
export RUST_BACKTRACE=1
nohup "$API_BIN" >> "$LOG_FILE" 2>&1 &
API_PID=$!
echo "$API_PID" > "$PID_FILE"

sleep 1
if kill -0 "$API_PID" 2>/dev/null; then
    echo "✓ trusty-api started (PID $API_PID)"
    echo "  http://localhost:$PORT"
    echo "  https://izzie.ngrok.dev  (if ngrok is running)"
else
    echo "✗ trusty-api failed to start — check $LOG_FILE"
    rm -f "$PID_FILE"
    exit 1
fi
