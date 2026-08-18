#!/usr/bin/env bash
# Generate SHA256SUMS and SHA512SUMS checksum manifests
# Usage: generate-checksums.sh [dir]
#
# Creates checksums for all release artifacts (excludes signatures and checksum files)
set -euo pipefail

DIR=${1:-dist/release}

if [ ! -d "$DIR" ]; then
	echo "Error: Directory $DIR does not exist"
	exit 1
fi

cd "$DIR"

echo "Generating checksums in $DIR..."

# Archives, SBOM/metadata, licenses, copied release notes.
# No FFI headers or committed static libraries.
CHECKSUM_PATTERNS=(
	'*.tar.gz'
	'*.zip'
	'*.json'
	'LICENSE-*'
	'release-notes-*.md'
)

FIND_ARGS=()
for pattern in "${CHECKSUM_PATTERNS[@]}"; do
	if [ ${#FIND_ARGS[@]} -gt 0 ]; then
		FIND_ARGS+=("-o")
	fi
	FIND_ARGS+=("-name" "$pattern")
done

find . -maxdepth 1 -type f \( "${FIND_ARGS[@]}" \) \
	! -name 'SHA*' \
	! -name '*.minisig' \
	! -name '*.asc' \
	! -name '*.pub' \
	-print0 | sort -z | xargs -0 shasum -a 256 >SHA256SUMS

echo "Generated SHA256SUMS:"
cat SHA256SUMS

find . -maxdepth 1 -type f \( "${FIND_ARGS[@]}" \) \
	! -name 'SHA*' \
	! -name '*.minisig' \
	! -name '*.asc' \
	! -name '*.pub' \
	-print0 | sort -z | xargs -0 shasum -a 512 >SHA512SUMS

echo ""
echo "Generated SHA512SUMS"
echo "[ok] Checksums generated"
