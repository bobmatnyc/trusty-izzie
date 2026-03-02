#!/usr/bin/env bash
# scripts/version-bump.sh — Semantic version bump for trusty-izzie.
#
# Usage:
#   version-bump.sh patch     # 0.1.0 → 0.1.1
#   version-bump.sh minor     # 0.1.0 → 0.2.0
#   version-bump.sh major     # 0.1.0 → 1.0.0
#   version-bump.sh 0.3.0     # set exact version
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
CARGO_TOML="$PROJECT_DIR/Cargo.toml"

cd "$PROJECT_DIR"

# --- Read current version ---
CURRENT=$(grep -m1 '^version = ' "$CARGO_TOML" | sed 's/version = "\(.*\)"/\1/')
if [[ -z "$CURRENT" ]]; then
    echo "✗ Could not read version from $CARGO_TOML"
    exit 1
fi

IFS='.' read -r MAJ MIN PAT <<< "$CURRENT"

BUMP="${1:-patch}"
case "$BUMP" in
    major)
        NEW_VERSION="$((MAJ + 1)).0.0"
        ;;
    minor)
        NEW_VERSION="${MAJ}.$((MIN + 1)).0"
        ;;
    patch)
        NEW_VERSION="${MAJ}.${MIN}.$((PAT + 1))"
        ;;
    [0-9]*)
        NEW_VERSION="$BUMP"
        ;;
    *)
        echo "Usage: version-bump.sh [major|minor|patch|X.Y.Z]"
        exit 1
        ;;
esac

echo "▶ Bumping version: $CURRENT → $NEW_VERSION"

# --- Update Cargo.toml workspace version ---
# Match only the first `version = "..."` line (the workspace version)
if [[ "$(uname)" == "Darwin" ]]; then
    sed -i '' "s/^version = \"${CURRENT}\"/version = \"${NEW_VERSION}\"/" "$CARGO_TOML"
else
    sed -i "s/^version = \"${CURRENT}\"/version = \"${NEW_VERSION}\"/" "$CARGO_TOML"
fi

# Verify the change
UPDATED=$(grep -m1 '^version = ' "$CARGO_TOML" | sed 's/version = "\(.*\)"/\1/')
if [[ "$UPDATED" != "$NEW_VERSION" ]]; then
    echo "✗ Failed to update version in Cargo.toml"
    exit 1
fi

# --- Update Cargo.lock ---
echo "  Updating Cargo.lock…"
cargo update -p trusty-cli 2>/dev/null || cargo generate-lockfile

# --- Commit ---
git add "$CARGO_TOML" Cargo.lock
git commit -m "chore: bump version to v${NEW_VERSION}

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"

echo "✓ Version bumped to v${NEW_VERSION}"
echo ""
echo "  Next steps:"
echo "    make tag      → create and push git tag v${NEW_VERSION}"
echo "    make release  → tag + push + build release binary"
