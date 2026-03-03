#!/bin/bash
set -euo pipefail

LAUNCH_AGENTS="$HOME/Library/LaunchAgents"
TRUSTY_BIN="$HOME/.local/bin"

echo "==> Unloading launchd services"
for plist in "$LAUNCH_AGENTS"/com.trusty-izzie.*.plist; do
    if [ -f "$plist" ]; then
        launchctl unload "$plist" 2>/dev/null && echo "    unloaded $(basename "$plist")" || true
        rm "$plist"
        echo "    removed $(basename "$plist")"
    fi
done

echo ""
read -p "Remove trusty-izzie binaries from $TRUSTY_BIN? [y/N] " -n 1 -r
echo
if [[ $REPLY =~ ^[Yy]$ ]]; then
    rm -f "$TRUSTY_BIN"/trusty-daemon "$TRUSTY_BIN"/trusty-api "$TRUSTY_BIN"/trusty-cli "$TRUSTY_BIN"/trusty-telegram "$TRUSTY_BIN"/trusty
    echo "    binaries removed"
fi

echo ""
read -p "DANGER: Wipe all trusty-izzie data at ~/.local/share/trusty-izzie? [y/N] " -n 1 -r
echo
if [[ $REPLY =~ ^[Yy]$ ]]; then
    echo "DATA NOT DELETED — manual deletion required for safety"
    echo "Run: rm -rf ~/.local/share/trusty-izzie"
fi

echo "Uninstall complete."
