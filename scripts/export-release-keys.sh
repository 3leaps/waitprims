#!/usr/bin/env bash
# Export public signing keys to release directory
# Usage: export-release-keys.sh [dir]
#
# Environment variables:
#   WAITPRIMS_MINISIGN_PUB  - Path to minisign public key (or derives from WAITPRIMS_MINISIGN_KEY)
#   WAITPRIMS_MINISIGN_KEY  - Path to minisign secret key (used to derive public key location)
#   WAITPRIMS_PGP_KEY_ID    - PGP key ID for optional export (optional)
#   WAITPRIMS_GPG_HOMEDIR   - Custom GPG home directory (optional)
set -euo pipefail

DIR=${1:-dist/release}

if [ ! -d "$DIR" ]; then
	echo "Error: Directory $DIR does not exist"
	exit 1
fi

echo "Exporting public keys to $DIR..."

echo ""
echo "=== Minisign Public Key ==="

MINISIGN_PUB="${WAITPRIMS_MINISIGN_PUB:-}"

if [ -z "$MINISIGN_PUB" ] && [ -n "${WAITPRIMS_MINISIGN_KEY:-}" ]; then
	MINISIGN_PUB="${WAITPRIMS_MINISIGN_KEY%.key}.pub"
fi

if [ -n "$MINISIGN_PUB" ] && [ -f "$MINISIGN_PUB" ]; then
	cp "$MINISIGN_PUB" "$DIR/waitprims-minisign.pub"
	echo "[ok] Exported $DIR/waitprims-minisign.pub"
	cat "$DIR/waitprims-minisign.pub"
else
	echo "[!!] Minisign public key not found"
	echo "Set WAITPRIMS_MINISIGN_PUB or ensure .pub file exists alongside .key"
fi

if [ -n "${WAITPRIMS_PGP_KEY_ID:-}" ]; then
	echo ""
	echo "=== PGP Public Key ==="

	GPG_OPTS=()
	if [ -n "${WAITPRIMS_GPG_HOMEDIR:-}" ]; then
		GPG_OPTS+=("--homedir" "$WAITPRIMS_GPG_HOMEDIR")
	fi

	gpg "${GPG_OPTS[@]}" \
		--armor \
		--export "$WAITPRIMS_PGP_KEY_ID" \
		>"$DIR/waitprims-release-signing-key.asc"

	echo "[ok] Exported $DIR/waitprims-release-signing-key.asc"
else
	echo ""
	echo "[--] PGP key export skipped (WAITPRIMS_PGP_KEY_ID not set)"
fi

echo ""
echo "[ok] Key export complete"
