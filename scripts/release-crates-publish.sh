#!/usr/bin/env bash
# One separately cued maintainer upload; never a batch publication loop.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd -P)"
cd "$root"
crate="${1:?one approved crate required}"
scratch="$(mktemp)"
trap 'rm -f "$scratch"' EXIT
WAITPRIMS_CRATE_ANCHOR_OUT="$scratch" "$root/scripts/release-crates-dry-run.sh" "$crate"
object="$(awk -F= '$1=="object" {print $2}' "$scratch")"
commit="$(awk -F= '$1=="commit" {print $2}' "$scratch")"
[[ "$object" =~ ^[0-9a-f]{40}$ && "$commit" =~ ^[0-9a-f]{40}$ ]] || {
    echo 'error: missing registry dry-run identity' >&2
    exit 1
}
"$root/scripts/check-registry-config.py"
WAITPRIMS_EXPECTED_TAG_OBJECT="$object" WAITPRIMS_EXPECTED_COMMIT="$commit" "$root/scripts/release-verify-published-tag.sh"
cargo +1.88.0 publish --registry crates-io --locked -p "$crate"
version="$(cat VERSION)"
cargo +1.88.0 info --registry crates-io "$crate@$version"
