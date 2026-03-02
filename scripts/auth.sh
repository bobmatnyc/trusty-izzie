#!/usr/bin/env bash
# scripts/auth.sh — Run the Google OAuth2 login flow via the trusty CLI.
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

echo "▶ Starting Google OAuth2 login…"
echo "  A browser window will open — log in with bob@matsuoka.com"
echo ""
exec "$CLI_BIN" auth "$@"
