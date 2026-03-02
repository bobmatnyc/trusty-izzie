#!/usr/bin/env bash
# scripts/build.sh — Build trusty-izzie binaries.
# Usage: build.sh [release|dev]
set -euo pipefail

PROFILE="${1:-release}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

cd "$PROJECT_DIR"

# Load .env if present
if [[ -f .env ]]; then
    set -a
    # shellcheck disable=SC1091
    source .env
    set +a
fi

if [[ "$PROFILE" == "release" ]]; then
    echo "▶ Building trusty-izzie (release)…"
    cargo build --release --bins
    echo "✓ Release binaries in target/release/"
else
    echo "▶ Building trusty-izzie (dev)…"
    cargo build --bins
    echo "✓ Dev binaries in target/debug/"
fi
