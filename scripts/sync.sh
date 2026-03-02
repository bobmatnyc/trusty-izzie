#!/usr/bin/env bash
# scripts/sync.sh — Trigger an immediate Gmail sync via the CLI.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

if [[ -x "$PROJECT_DIR/target/release/trusty" ]]; then
    CLI_BIN="$PROJECT_DIR/target/release/trusty"
elif [[ -x "$PROJECT_DIR/target/debug/trusty" ]]; then
    CLI_BIN="$PROJECT_DIR/target/debug/trusty"
else
    echo "✗ CLI binary not found. Run 'make build' first."
    exit 1
fi

if [[ -f "$PROJECT_DIR/.env" ]]; then
    set -a
    # shellcheck disable=SC1091
    source "$PROJECT_DIR/.env"
    set +a
fi

echo "▶ Triggering Gmail sync…"
exec "$CLI_BIN" sync "$@"
