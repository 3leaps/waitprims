#!/usr/bin/env bash
# Sign release checksum manifests with minisign (and optionally PGP)
# Usage: sign-release-assets.sh <tag> [dir]
#
# Environment variables:
#   WAITPRIMS_MINISIGN_KEY  - Path to minisign secret key (required)
#   WAITPRIMS_PGP_KEY_ID    - PGP key ID for optional GPG signing (optional)
#   WAITPRIMS_GPG_HOMEDIR   - Custom GPG home directory (optional)
#
# Requires: minisign, optionally gpg
set -euo pipefail

TAG=${1:?"usage: sign-release-assets.sh <tag> [dir]"}
DIR=${2:-dist/release}

if [ ! -d "$DIR" ]; then
	echo "Error: Directory $DIR does not exist"
	exit 1
fi

if [ -z "${WAITPRIMS_MINISIGN_KEY:-}" ]; then
	echo "Error: WAITPRIMS_MINISIGN_KEY environment variable not set"
	echo "Load the secure release-signing environment and retry."
	exit 1
fi

if [ ! -f "$WAITPRIMS_MINISIGN_KEY" ]; then
	echo "Error: Configured minisign key is not a readable file"
	exit 1
fi

cd "$DIR"

MISSING=0
for manifest in SHA256SUMS SHA512SUMS; do
	if [ ! -f "$manifest" ]; then
		echo "Error: $manifest not found in $DIR"
		MISSING=$((MISSING + 1))
	fi
done
if [ $MISSING -gt 0 ]; then
	echo ""
	echo "Did you forget to run checksums first?"
	echo "  make release-checksums"
	exit 1
fi

echo "Signing release $TAG..."

echo ""
echo "=== Minisign Signatures ==="

for manifest in SHA256SUMS SHA512SUMS; do
	if [ -f "$manifest" ]; then
		echo "Signing $manifest with minisign..."
		minisign -S -s "$WAITPRIMS_MINISIGN_KEY" \
			-m "$manifest" \
			-t "waitprims $TAG - $(date -u +%Y-%m-%dT%H:%M:%SZ)" \
			-x "${manifest}.minisig"
		echo "[ok] Created ${manifest}.minisig"
	fi
done

if [ -n "${WAITPRIMS_PGP_KEY_ID:-}" ]; then
	echo ""
	echo "=== PGP Signatures ==="

	GPG_OPTS=()
	if [ -n "${WAITPRIMS_GPG_HOMEDIR:-}" ]; then
		GPG_OPTS+=("--homedir" "$WAITPRIMS_GPG_HOMEDIR")
	fi

	for manifest in SHA256SUMS SHA512SUMS; do
		if [ -f "$manifest" ]; then
			echo "Signing $manifest with PGP..."
			gpg "${GPG_OPTS[@]}" \
				--armor \
				--detach-sign \
				--local-user "$WAITPRIMS_PGP_KEY_ID" \
				--output "${manifest}.asc" \
				"$manifest"
			echo "[ok] Created ${manifest}.asc"
		fi
	done
else
	echo ""
	echo "[--] PGP signing skipped (WAITPRIMS_PGP_KEY_ID not set)"
fi

echo ""
echo "[ok] Signing complete"
ls -la ./*.minisig ./*.asc 2>/dev/null || true
