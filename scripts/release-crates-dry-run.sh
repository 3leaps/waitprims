#!/usr/bin/env bash
# One unpatched registry dry run after the approved signed tag; never uploads.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd -P)"
cd "$root"
source "$root/scripts/release-common.sh"
crate="${1:?publishable crate required}"
"$root/scripts/check-registry-config.py"
"$root/scripts/release-crates.py" check
grep -Fxq "$crate" config/release/publishable-crates.txt || {
    echo 'error: unpublished or unknown crate' >&2
    exit 1
}
tag="$(release_tag)"
version="${tag#v}"
scratch="$(mktemp)"
trap 'rm -f "$scratch"' EXIT
WAITPRIMS_ANCHOR_OUT="$scratch" "$root/scripts/release-verify-published-tag.sh"
commit="$(awk -F= '$1=="commit" {print $2}' "$scratch")"
[[ "$(git rev-parse HEAD)" == "$commit" && -z "$(git status --porcelain --untracked-files=all)" ]] || {
    echo 'error: registry check needs a clean checkout of the verified commit' >&2
    exit 1
}
[[ "$(release_version)" == "$version" ]] || {
    echo 'error: registry version mismatch' >&2
    exit 1
}
# Every previous crate (including fs's testkit dev dependency) must be indexed.
while IFS= read -r predecessor; do
    [[ "$predecessor" != "$crate" ]] || break
    cargo +1.88.0 info --registry crates-io "$predecessor@$version" >/dev/null
done <config/release/publishable-crates.txt
cargo +1.88.0 publish --registry crates-io --dry-run --locked -p "$crate"

object="$(awk -F= '$1=="object" {print $2}' "$scratch")"
WAITPRIMS_EXPECTED_TAG_OBJECT="$object" WAITPRIMS_EXPECTED_COMMIT="$commit" \
    "$root/scripts/release-verify-published-tag.sh"
if [[ -n "${WAITPRIMS_CRATE_ANCHOR_OUT:-}" ]]; then
    cp "$scratch" "$WAITPRIMS_CRATE_ANCHOR_OUT"
fi
