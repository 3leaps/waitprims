#!/usr/bin/env bash
# Verify both manifests against the exact signable inventory.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd -P)"
source "$root/scripts/release-common.sh"
directory="${1:-dist/release}"
files=()
while IFS= read -r name; do files+=("$name"); done < <(release_signable_assets)
python3 - "$directory" "${files[@]}" <<'PY'
import hashlib
import pathlib
import re
import sys
directory = pathlib.Path(sys.argv[1])
expected = sorted(sys.argv[2:])
for manifest, algorithm, width in [('SHA256SUMS', 'sha256', 64), ('SHA512SUMS', 'sha512', 128)]:
    path = directory / manifest
    if not path.is_file() or path.is_symlink():
        sys.exit('error: unsafe or absent checksum manifest')
    entries = {}
    for line in path.read_text().splitlines():
        digest, sep, name = line.partition('  ')
        if not sep or not re.fullmatch('[0-9a-f]{' + str(width) + '}', digest) or name in entries:
            sys.exit('error: malformed or duplicate checksum entry')
        entries[name] = digest
    if sorted(entries) != expected:
        sys.exit('error: checksum inventory differs from exact release set')
    for name in expected:
        asset = directory / name
        if not asset.is_file() or asset.is_symlink() or not asset.stat().st_size:
            sys.exit('error: missing or unsafe checksummed asset')
        if hashlib.new(algorithm, asset.read_bytes()).hexdigest() != entries[name]:
            sys.exit('error: checksum mismatch for ' + name)
print('[ok] both checksum manifests match the exact release set')
PY
