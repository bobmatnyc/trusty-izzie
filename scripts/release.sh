#!/usr/bin/env bash
# scripts/release.sh — Full semantic release: build + tag + push.
#
# Usage:
#   release.sh              # bump patch, tag, push
#   release.sh minor        # bump minor, tag, push
#   release.sh major        # bump major, tag, push
#   release.sh --tag-only   # just tag current version without bumping
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
CARGO_TOML="$PROJECT_DIR/Cargo.toml"

cd "$PROJECT_DIR"

TAG_ONLY=false
BUMP="patch"
for arg in "$@"; do
    case "$arg" in
        --tag-only) TAG_ONLY=true ;;
        major|minor|patch) BUMP="$arg" ;;
        [0-9]*) BUMP="$arg" ;;
    esac
done

# --- Ensure clean working tree ---
if [[ -n "$(git status --porcelain)" ]]; then
    echo "✗ Working tree is not clean. Commit or stash changes first."
    git status --short
    exit 1
fi

# --- Version bump (unless --tag-only) ---
if [[ "$TAG_ONLY" == "false" ]]; then
    bash "$SCRIPT_DIR/version-bump.sh" "$BUMP"
fi

# --- Read version after bump ---
VERSION=$(grep -m1 '^version = ' "$CARGO_TOML" | sed 's/version = "\(.*\)"/\1/')
TAG="v${VERSION}"

echo ""
echo "▶ Releasing $TAG"

# --- Build release binaries ---
echo "  Building release binaries…"
cargo build --release --bins

# --- Generate build info ---
GIT_HASH=$(git rev-parse --short=8 HEAD)
BUILD_DATE=$(date -u '+%Y-%m-%dT%H:%M:%SZ')

# Write BUILD file for tracking
cat > "$PROJECT_DIR/BUILD" << BUILDEOF
version = "${VERSION}"
git_hash = "${GIT_HASH}"
build_date = "${BUILD_DATE}"
BUILDEOF
git add "$PROJECT_DIR/BUILD"
git commit -m "build: ${TAG} (${GIT_HASH})" 2>/dev/null || true

# --- Tag ---
if git tag -l | grep -q "^${TAG}$"; then
    echo "  Tag ${TAG} already exists — skipping tag creation"
else
    git tag -a "${TAG}" -m "Release ${TAG}

Built: ${BUILD_DATE}
Commit: ${GIT_HASH}"
    echo "  Created tag ${TAG}"
fi

# --- Push ---
echo "  Pushing to origin…"
git push origin main
git push origin "${TAG}"

echo ""
echo "✓ Released ${TAG}"
echo "  Binaries: target/release/{trusty,trusty-daemon,trusty-api,trusty-telegram}"
echo "  Tag:      ${TAG} @ ${GIT_HASH}"
