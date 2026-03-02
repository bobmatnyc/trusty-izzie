#!/usr/bin/env bash
# One-time LanceDB schema migration: adds canonical `id` column to Python-migrated tables.
# Safe to re-run (overwrites in place).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
cd "$PROJECT_DIR"

# Load .env if present
if [[ -f .env ]]; then
    # Export only lines that look like KEY=VALUE (skip comments and blanks)
    set -a
    # shellcheck disable=SC1091
    source .env
    set +a
fi

echo "Building migration binary..."
cargo build --release -p trusty-migrate

echo "Running migration..."
./target/release/trusty-migrate

echo "Migration complete. Run 'cargo run --release -p trusty-cli -- entity list --limit 5' to verify."
