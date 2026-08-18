#!/usr/bin/env bash
# Generate SHA256SUMS and SHA512SUMS checksum manifests
# Usage: generate-checksums.sh [dir] [tag]
#
# Checksums only this tag's artifacts, licenses, and the copied
# RELEASE_NOTES.md payload (release-notes-<tag>.md). Leftover files
# from an earlier cut are ignored (and reported).
set -euo pipefail

DIR=${1:-dist/release}
TAG=${2:-${WAITPRIMS_RELEASE_TAG:-}}

if [ ! -d "$DIR" ]; then
	echo "Error: Directory $DIR does not exist"
	exit 1
fi

if [ -z "$TAG" ] || [ "$TAG" = "v" ]; then
	echo "Error: No release tag. Pass the tag or set WAITPRIMS_RELEASE_TAG=vX.Y.Z"
	exit 1
fi

VERSION="${TAG#v}"
NOTES="release-notes-${TAG}.md"

cd "$DIR"

if [ ! -f "$NOTES" ]; then
	echo "Error: $NOTES not in $DIR"
	echo "Copy RELEASE_NOTES.md before checksums so it is in the signed set:"
	echo "  make release-notes"
	exit 1
fi

echo "Generating checksums in $DIR for $TAG..."

CHECKSUM_FILES=()
for f in LICENSE-* \
	"$NOTES" \
	"sbom-${VERSION}.cdx.json" \
	"waitprims-${VERSION}-"*.tar.gz \
	"waitprims-${VERSION}-"*.zip; do
	if [ -f "$f" ]; then
		CHECKSUM_FILES+=("$f")
	fi
done

if [ ${#CHECKSUM_FILES[@]} -eq 0 ]; then
	echo "Error: no checksum candidates for $TAG in $DIR"
	exit 1
fi

printf '%s\n' "${CHECKSUM_FILES[@]}" | LC_ALL=C sort | xargs shasum -a 256 >SHA256SUMS
printf '%s\n' "${CHECKSUM_FILES[@]}" | LC_ALL=C sort | xargs shasum -a 512 >SHA512SUMS

echo "Generated SHA256SUMS:"
cat SHA256SUMS

leftovers=0
for f in release-notes-*.md sbom-*.json waitprims-*.tar.gz waitprims-*.zip; do
	[ -e "$f" ] || continue
	keep=0
	for listed in "${CHECKSUM_FILES[@]}"; do
		if [ "$f" = "$listed" ]; then
			keep=1
			break
		fi
	done
	if [ "$keep" -eq 0 ]; then
		if [ "$leftovers" -eq 0 ]; then
			echo ""
			echo "[--] leftover files not in this tag's checksum set:"
		fi
		echo "    $f"
		leftovers=1
	fi
done
if [ "$leftovers" -eq 1 ]; then
	echo "    run: make release-clean && make release-download && make release-notes"
fi

echo ""
echo "Generated SHA512SUMS"
echo "[ok] Checksums generated"
