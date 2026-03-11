#!/usr/bin/env bash
# scripts/telegram-start.sh — Start the Telegram bot in the background.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

TG_PID_FILE="/tmp/trusty-telegram.pid"
TG_LOG_FILE="/tmp/trusty-telegram.log"

if [[ -x "$PROJECT_DIR/target/release/trusty-telegram" ]]; then
    TG_BIN="$PROJECT_DIR/target/release/trusty-telegram"
elif [[ -x "$PROJECT_DIR/target/debug/trusty-telegram" ]]; then
    TG_BIN="$PROJECT_DIR/target/debug/trusty-telegram"
else
    echo "✗ trusty-telegram binary not found."
    echo "  Run 'make build' first."
    exit 1
fi

if [[ -f "$TG_PID_FILE" ]]; then
    OLD_PID=$(cat "$TG_PID_FILE")
    if kill -0 "$OLD_PID" 2>/dev/null; then
        echo "▸ trusty-telegram is already running (PID $OLD_PID)"
        exit 0
    fi
    rm -f "$TG_PID_FILE"
fi

if [[ -f "$PROJECT_DIR/.env" ]]; then
    set -a
    # shellcheck disable=SC1091
    source "$PROJECT_DIR/.env"
    set +a
fi

echo "▶ Starting trusty-telegram bot…"
echo "  Log → $TG_LOG_FILE"

export RUST_BACKTRACE=1
nohup "$TG_BIN" start \
    --webhook-url "${TRUSTY_PUBLIC_URL:-https://localhost:3456}/webhook/telegram" \
    --port 3456 \
    >> "$TG_LOG_FILE" 2>&1 &
TG_PID=$!
echo "$TG_PID" > "$TG_PID_FILE"

sleep 1
if kill -0 "$TG_PID" 2>/dev/null; then
    echo "✓ trusty-telegram started (PID $TG_PID)"
    echo "  tail -f $TG_LOG_FILE  to follow logs"
    echo "  make telegram-stop    to stop"
else
    echo "✗ trusty-telegram failed to start — check $TG_LOG_FILE"
    rm -f "$TG_PID_FILE"
    exit 1
fi
