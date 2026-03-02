#!/usr/bin/env bash
# scripts/chat.sh — Run an interactive chat session via the trusty CLI.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

# Prefer release, fall back to dev build
if [[ -x "$PROJECT_DIR/target/release/trusty" ]]; then
    CLI_BIN="$PROJECT_DIR/target/release/trusty"
elif [[ -x "$PROJECT_DIR/target/debug/trusty" ]]; then
    CLI_BIN="$PROJECT_DIR/target/debug/trusty"
else
    echo "▶ CLI binary not found — building dev…"
    cd "$PROJECT_DIR"
    cargo build -p trusty-cli 2>&1
    CLI_BIN="$PROJECT_DIR/target/debug/trusty"
fi

# Load .env
if [[ -f "$PROJECT_DIR/.env" ]]; then
    set -a
    # shellcheck disable=SC1091
    source "$PROJECT_DIR/.env"
    set +a
fi

echo "▶ Starting trusty chat (Ctrl-C or /quit to exit)…"
echo ""
exec "$CLI_BIN" chat "$@"
