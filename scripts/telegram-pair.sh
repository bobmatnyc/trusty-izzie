#!/usr/bin/env bash
# scripts/telegram-pair.sh — Pair a Telegram bot token interactively.
#
# Usage:
#   bash scripts/telegram-pair.sh
#   TELEGRAM_TOKEN=<token> TELEGRAM_USERS=123456,789012 bash scripts/telegram-pair.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

if [[ -x "$PROJECT_DIR/target/release/trusty-telegram" ]]; then
    TG_BIN="$PROJECT_DIR/target/release/trusty-telegram"
elif [[ -x "$PROJECT_DIR/target/debug/trusty-telegram" ]]; then
    TG_BIN="$PROJECT_DIR/target/debug/trusty-telegram"
else
    echo "✗ trusty-telegram binary not found."
    echo "  Run 'make build' first."
    exit 1
fi

if [[ -f "$PROJECT_DIR/.env" ]]; then
    set -a
    # shellcheck disable=SC1091
    source "$PROJECT_DIR/.env"
    set +a
fi

# Accept token from env or prompt interactively
TOKEN="${TELEGRAM_TOKEN:-}"
if [[ -z "$TOKEN" ]]; then
    echo "Telegram Bot Token Pairing"
    echo "──────────────────────────"
    echo "  1. Open Telegram → search @BotFather"
    echo "  2. Send /newbot and follow prompts"
    echo "  3. Copy the token (looks like: 123456789:AAHdqTcvCH1vGWJxfSeofSh0K4Kvh21)"
    echo ""
    read -r -p "Paste your bot token: " TOKEN
fi

# Accept allowed users from env or prompt
USERS="${TELEGRAM_USERS:-}"
if [[ -z "$USERS" ]]; then
    echo ""
    echo "Allowed Telegram User IDs (comma-separated, leave blank to allow everyone):"
    echo "  To find your ID: message @userinfobot on Telegram"
    read -r -p "Allowed user IDs: " USERS
fi

# Build the pair command
PAIR_ARGS=("pair" "--token" "$TOKEN")
if [[ -n "$USERS" ]]; then
    PAIR_ARGS+=("--allowed-users" "$USERS")
fi

echo ""
echo "▶ Storing bot token…"
"$TG_BIN" "${PAIR_ARGS[@]}"

echo ""
echo "▶ Next steps:"
echo "  make telegram       — start the bot"
echo "  make telegram-stop  — stop the bot"
echo ""
echo "  Or test with: TELEGRAM_BOT_TOKEN=\$TOKEN target/release/trusty-telegram start"
