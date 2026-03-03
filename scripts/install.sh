#!/bin/bash
set -euo pipefail

# Paths
TRUSTY_DATA="$HOME/.local/share/trusty-izzie"
TRUSTY_LOGS="$TRUSTY_DATA/logs"
TRUSTY_BIN="$HOME/.local/bin"
LAUNCH_AGENTS="$HOME/Library/LaunchAgents"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
ENV_FILE="$TRUSTY_DATA/.env"

echo "==> Building trusty-izzie (release)"
cd "$REPO_ROOT"
cargo build --release

echo "==> Creating directories"
mkdir -p "$TRUSTY_LOGS" "$TRUSTY_BIN" "$LAUNCH_AGENTS"

echo "==> Installing binaries"
for bin in trusty-daemon trusty-api trusty-cli; do
    if [ -f "target/release/$bin" ]; then
        cp "target/release/$bin" "$TRUSTY_BIN/"
        echo "    installed $bin"
    fi
done
# trusty-telegram may not exist yet — install if present
if [ -f "target/release/trusty-telegram" ]; then
    cp "target/release/trusty-telegram" "$TRUSTY_BIN/"
    echo "    installed trusty-telegram"
fi

echo "==> Installing launchd plists"
for plist in "$REPO_ROOT"/launchd/com.trusty-izzie.*.plist; do
    label=$(basename "$plist" .plist)
    dest="$LAUNCH_AGENTS/$label.plist"

    # Substitute HOME placeholder
    sed "s|__HOME__|$HOME|g" "$plist" > "$dest"

    # Note env vars from .env if present
    if [ -f "$ENV_FILE" ]; then
        echo "    found .env at $ENV_FILE"
    fi

    # Unload if running, then load
    launchctl unload "$dest" 2>/dev/null || true
    launchctl load "$dest"
    echo "    loaded $label"
done

echo ""
echo "==> Service status:"
sleep 2
launchctl list | grep trusty-izzie | awk '{printf "  %-45s PID=%-8s Exit=%s\n", $3, $1, $2}' || echo "  (no services found)"

echo ""
echo "trusty-izzie installed successfully!"
echo "Logs: $TRUSTY_LOGS"
