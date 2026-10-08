#!/usr/bin/env bash
# Install a public anchor pair; rotation requires an explicit reviewed cue.
set -euo pipefail
staging="${1:?staging directory required}"
dest="${2:?destination required}"
shift 2
for name in expected-fingerprints.txt expected-fingerprints.ndjson; do
    [[ -s "$staging/$name" && ! -L "$staging/$name" ]] || {
        echo 'error: incomplete anchor pair' >&2
        exit 1
    }
done
[[ ! -L "$dest" ]] || {
    echo 'error: unsafe anchor directory' >&2
    exit 1
}
for name in expected-fingerprints.txt expected-fingerprints.ndjson; do
    [[ ! -L "$dest/$name" ]] || {
        echo 'error: unsafe existing anchor' >&2
        exit 1
    }
    if [[ -e "$dest/$name" && "${WAITPRIMS_ALLOW_ANCHOR_ROTATION:-}" != 1 ]]; then
        echo 'error: anchors exist; explicit reviewed rotation required' >&2
        exit 1
    fi
done
mkdir -p "$dest"
backup="$(mktemp -d)"
for name in expected-fingerprints.txt expected-fingerprints.ndjson; do
    if [[ -f "$dest/$name" ]]; then cp "$dest/$name" "$backup/$name"; fi
done
rollback() {
    trap - EXIT INT TERM HUP
    for name in expected-fingerprints.txt expected-fingerprints.ndjson; do
        rm -f "$dest/$name.new"
        if [[ -f "$backup/$name" ]]; then mv -f "$backup/$name" "$dest/$name"; else rm -f "$dest/$name"; fi
    done
    rm -rf "$backup"
}
trap rollback EXIT
trap 'rollback; exit 130' INT
trap 'rollback; exit 143' TERM
trap 'rollback; exit 129' HUP
for name in expected-fingerprints.txt expected-fingerprints.ndjson; do
    cp "$staging/$name" "$dest/$name.new"
done
mv -f "$dest/expected-fingerprints.ndjson.new" "$dest/expected-fingerprints.ndjson"
if [[ "${WAITPRIMS_TEST_FAIL_ANCHOR_INSTALL:-}" == 1 ]]; then
    echo 'error: interrupted pair installation' >&2
    exit 1
fi
mv -f "$dest/expected-fingerprints.txt.new" "$dest/expected-fingerprints.txt"
if [[ "$#" -gt 0 ]]; then
    "$@"
fi
trap - EXIT INT TERM HUP
rm -rf "$backup"
