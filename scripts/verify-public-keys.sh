#!/usr/bin/env bash
# Verify that exported keys contain only public material (no secrets)
# Usage: verify-public-keys.sh [dir]
#
# Critical safety check before uploading to GitHub
set -euo pipefail

DIR=${1:-dist/release}

if [ ! -d "$DIR" ]; then
	echo "Error: Directory $DIR does not exist"
	exit 1
fi

cd "$DIR"

echo "Verifying public keys contain no secret material..."

ERRORS=0

echo ""
echo "=== Minisign Key Check ==="

if [ -f "waitprims-minisign.pub" ]; then
	if grep -qi "secret" "waitprims-minisign.pub"; then
		echo "[!!] DANGER: waitprims-minisign.pub may contain secret key material!"
		ERRORS=$((ERRORS + 1))
	elif grep -q "^untrusted comment:" "waitprims-minisign.pub"; then
		echo "[ok] waitprims-minisign.pub appears to be a valid public key"
	else
		echo "[!!] waitprims-minisign.pub has unexpected format"
		ERRORS=$((ERRORS + 1))
	fi
else
	echo "[--] waitprims-minisign.pub not found"
fi

echo ""
echo "=== PGP Key Check ==="

if [ -f "waitprims-release-signing-key.asc" ]; then
	if grep -q "PRIVATE KEY BLOCK" "waitprims-release-signing-key.asc"; then
		echo "[!!] DANGER: waitprims-release-signing-key.asc contains PRIVATE KEY!"
		ERRORS=$((ERRORS + 1))
	elif grep -q "PUBLIC KEY BLOCK" "waitprims-release-signing-key.asc"; then
		echo "[ok] waitprims-release-signing-key.asc is a public key"

		GNUPGHOME=$(mktemp -d)
		export GNUPGHOME
		trap 'rm -rf "$GNUPGHOME"' EXIT

		if gpg --import waitprims-release-signing-key.asc 2>/dev/null; then
			echo "Key info:"
			gpg --list-keys 2>/dev/null | grep -A1 "^pub" || true
		fi
	else
		echo "[!!] waitprims-release-signing-key.asc has unexpected format"
		ERRORS=$((ERRORS + 1))
	fi
else
	echo "[--] waitprims-release-signing-key.asc not found"
fi

echo ""
if [ $ERRORS -eq 0 ]; then
	echo "[ok] Public key verification passed"
	exit 0
else
	echo "[!!] CRITICAL: Found $ERRORS potential secret key exposures!"
	echo "DO NOT upload these files to GitHub!"
	exit 1
fi
