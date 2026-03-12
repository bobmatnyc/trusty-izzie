#!/usr/bin/env bash
# Start trusty-slack bot
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

cd "$ROOT"
source .env 2>/dev/null || true

: "${SLACK_BOT_TOKEN:?SLACK_BOT_TOKEN must be set}"
: "${SLACK_SIGNING_SECRET:?SLACK_SIGNING_SECRET must be set}"
: "${OPENROUTER_API_KEY:?OPENROUTER_API_KEY must be set}"
: "${SLACK_PORT:=3457}"

exec cargo run --release --bin trusty-slack -- "$@"
