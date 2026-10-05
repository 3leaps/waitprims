#!/usr/bin/env bash
# Staged keys/anchors must equal the approved tagged commit's public material.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd -P)"
cd "$root"
source "$root/scripts/release-common.sh"
directory="${1:-dist/release}"
require_published_anchor "$directory" >/dev/null
commit="$(awk -F= '$1=="commit" {print $2}' "$directory.anchor")"
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
compare_blob() {
    local path="$1" file="$2" mode
    mode="$(git ls-tree "$commit" -- "$path" | awk '{print $1}')"
    [[ "$mode" == 100644 || "$mode" == 100755 ]] || {
        echo 'error: committed public file missing' >&2
        exit 1
    }
    git cat-file blob "$commit:$path" >"$scratch/blob"
    if [[ ! -f "$file" || -L "$file" ]] || ! cmp -s "$scratch/blob" "$file"; then
        echo 'error: staged public material differs from verified commit' >&2
        exit 1
    fi
}
for ext in txt ndjson; do compare_blob "keys/expected-fingerprints.$ext" "$directory/expected-fingerprints.$ext"; done
compare_blob docs/security/release-signing-keys.asc "$directory/waitprims-release-signing-key.asc"
compare_blob "docs/releases/$(release_tag).md" "$directory/release-notes-$(release_tag).md"
"$root/scripts/verify-public-keys.sh" "$directory"
