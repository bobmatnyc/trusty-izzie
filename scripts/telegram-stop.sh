#!/usr/bin/env bash
# scripts/telegram-stop.sh — Stop the Telegram bot.
set -euo pipefail

PID_FILE="/tmp/trusty-telegram.pid"

if [[ ! -f "$PID_FILE" ]]; then
    echo "▸ trusty-telegram is not running"
    exit 0
fi

PID=$(cat "$PID_FILE")
if ! kill -0 "$PID" 2>/dev/null; then
    echo "▸ trusty-telegram PID $PID no longer running (removing stale PID file)"
    rm -f "$PID_FILE"
    exit 0
fi

echo "▶ Stopping trusty-telegram (PID $PID)…"
kill -TERM "$PID"

for i in $(seq 1 10); do
    if ! kill -0 "$PID" 2>/dev/null; then break; fi
    sleep 0.5
done

if kill -0 "$PID" 2>/dev/null; then
    kill -KILL "$PID"
fi

rm -f "$PID_FILE"
echo "✓ trusty-telegram stopped"
