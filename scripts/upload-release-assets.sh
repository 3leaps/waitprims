#!/usr/bin/env bash
# Upload signed release assets to GitHub
# Usage: upload-release-assets.sh <tag> [dir]
#
# Uploads checksum files, signatures, public keys, and release notes.
# Release notes must already be in the signed checksum manifests
# (copied before `make release-checksums`). This script does not add
# unsigned notes after signing.
# Leaves the GitHub release as a draft. Publishing is a separate human step.
# Requires: gh CLI authenticated with write permissions
set -euo pipefail

TAG=${1:?"usage: upload-release-assets.sh <tag> [dir]"}
DIR=${2:-dist/release}

if [ ! -d "$DIR" ]; then
	echo "Error: Directory $DIR does not exist"
	exit 1
fi

cd "$DIR"

echo "Uploading signed assets for $TAG..."

REQUIRED_FILES=(
	"SHA256SUMS"
	"SHA256SUMS.minisig"
	"SHA512SUMS"
	"SHA512SUMS.minisig"
	"waitprims-minisign.pub"
)

for file in "${REQUIRED_FILES[@]}"; do
	if [ ! -f "$file" ]; then
		echo "Error: Required file missing: $file"
		echo "Run the signing workflow first:"
		echo "  make release-checksums"
		echo "  make release-sign"
		echo "  make release-export-keys"
		exit 1
	fi
done

UPLOAD_FILES=(
	"SHA256SUMS"
	"SHA256SUMS.minisig"
	"SHA512SUMS"
	"SHA512SUMS.minisig"
	"waitprims-minisign.pub"
)

for optional in "SHA256SUMS.asc" "SHA512SUMS.asc" "waitprims-release-signing-key.asc"; do
	if [ -f "$optional" ]; then
		UPLOAD_FILES+=("$optional")
	fi
done

RELEASE_NOTES="release-notes-${TAG}.md"
if [ -f "$RELEASE_NOTES" ]; then
	UPLOAD_FILES+=("$RELEASE_NOTES")
fi

for sbom in sbom-*.json; do
	if [ -f "$sbom" ]; then
		UPLOAD_FILES+=("$sbom")
	fi
done

echo "Uploading files:"
printf '  %s\n' "${UPLOAD_FILES[@]}"
echo ""

gh release upload "$TAG" "${UPLOAD_FILES[@]}" --clobber

if [ -f "$RELEASE_NOTES" ]; then
	echo ""
	echo "Updating release notes..."
	gh release edit "$TAG" --notes-file "$RELEASE_NOTES"
fi

echo ""
echo "[ok] Release $TAG assets uploaded (draft unchanged)"
echo "View at: https://github.com/3leaps/waitprims/releases/tag/$TAG"
echo "Publish when ready: gh release edit $TAG --draft=false"
