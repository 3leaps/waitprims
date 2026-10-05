#!/usr/bin/env bash
# Guard the reviewed release workflow's read-only artifact boundary.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd -P)"
python3 - "$root/.github/workflows/release.yml" <<'PY'
import pathlib
import re
import sys
text = pathlib.Path(sys.argv[1]).read_text()
def check(text):
    if re.search(r'(?m)^\s*[\w-]+:\s*(?:write|write-all)\b', text):
        raise ValueError('workflow write permission')
    if re.search(r'(?i)softprops|action-gh-release|gh\s+release|releases/(?:tags|latest)|curl[^\n]*releases', text):
        raise ValueError('workflow release creation or publication')
    uses = re.findall(r'(?m)^\s*uses:\s*(\S+)', text)
    allowed = {'actions/checkout', 'actions/upload-artifact', 'dtolnay/rust-toolchain'}
    for action in uses:
        repo, sep, ref = action.partition('@')
        if not sep or repo not in allowed or not re.fullmatch('[0-9a-f]{40}', ref):
            raise ValueError('unreviewed release action')
    if 'contents: read' not in text or 'release-identity.json' not in text:
        raise ValueError('missing artifact boundary')
check(text)
for mutation in [text + '\npermissions:\n  contents: write\n', text + '\npermissions: write-all\n', text + '\nrun: gh release create v1.2.3\n', text + '\nuses: softprops/action-gh-release@v2\n']:
    try:
        check(mutation)
    except ValueError:
        continue
    raise SystemExit('error: permission regression mutation passed')
print('[ok] read-only workflow boundary and mutation controls')
PY
