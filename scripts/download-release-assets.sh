#!/usr/bin/env bash
# Download release assets from GitHub
# Usage: download-release-assets.sh <tag> [dest_dir]
#
# Requires: gh CLI authenticated
set -euo pipefail

TAG=${1:?"usage: download-release-assets.sh <tag> [dest_dir]"}
DEST=${2:-dist/release}

echo "Downloading release assets for $TAG to $DEST..."

mkdir -p "$DEST"

# CLI archives, SBOM, and licenses. No FFI tarball, header, or bindings assets.
gh release download "$TAG" --dir "$DEST" --clobber \
	--pattern 'waitprims-*.tar.gz' \
	--pattern 'waitprims-*.zip' \
	--pattern 'sbom-*.json' \
	--pattern 'LICENSE-*'

echo "Downloaded to $DEST:"
ls -la "$DEST"
