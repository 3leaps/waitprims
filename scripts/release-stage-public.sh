#!/usr/bin/env bash
# Stage inert public material and cut notes from the verified release commit.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd -P)"
cd "$root"
source "$root/scripts/release-common.sh"
directory="${1:-dist/release}"
tag="$(release_tag)"
require_published_anchor "$directory" >/dev/null
commit="$(awk -F= '$1=="commit" {print $2}' "$directory.anchor")"
copy_blob() {
    local path="$1" output="$2" mode
    mode="$(git ls-tree "$commit" -- "$path" | awk '{print $1}')"
    [[ "$mode" == 100644 || "$mode" == 100755 ]] || {
        echo 'error: regular committed public file required' >&2
        exit 1
    }
    [[ ! -L "$output" ]] || {
        echo 'error: unsafe staging output' >&2
        exit 1
    }
    git cat-file blob "$commit:$path" >"$output"
    [[ -s "$output" ]] || {
        echo 'error: empty public output' >&2
        exit 1
    }
}
for ext in txt ndjson; do
    copy_blob "keys/expected-fingerprints.$ext" "$directory/expected-fingerprints.$ext"
done
copy_blob docs/security/release-signing-keys.asc "$directory/waitprims-release-signing-key.asc"
copy_blob "docs/releases/$tag.md" "$directory/release-notes-$tag.md"
: "${WAITPRIMS_MINISIGN_PUB:?explicit approved public minisign export required}"
[[ -f "$WAITPRIMS_MINISIGN_PUB" && -s "$WAITPRIMS_MINISIGN_PUB" && ! -L "$WAITPRIMS_MINISIGN_PUB" && ! -L "$directory/waitprims-minisign.pub" ]]
cp "$WAITPRIMS_MINISIGN_PUB" "$directory/waitprims-minisign.pub"
"$root/scripts/verify-public-keys.sh" "$directory"
