#!/usr/bin/env bash
# Trusted operator orchestration; only Cargo uses the verified tagged source.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd -P)"
cd "$root"
source "$root/scripts/release-common.sh"
mode=dry-run
if [[ "${1:-}" == --publish ]]; then
    mode=publish
    shift
fi
[[ $# == 1 ]] || {
    echo 'error: one publishable crate required' >&2
    exit 1
}
crate="$1"
grep -Fxq "$crate" "$root/config/release/publishable-crates.txt" || {
    echo 'error: unpublished or unknown crate' >&2
    exit 1
}
[[ -z "$(git status --porcelain --untracked-files=all)" ]] || {
    echo 'error: clean trusted operator checkout required' >&2
    exit 1
}
load_ceremony_anchor
object="$WAITPRIMS_EXPECTED_TAG_OBJECT"
commit="$WAITPRIMS_EXPECTED_COMMIT"
"$root/scripts/check-registry-config.py"
"$root/scripts/release-verify-published-tag.sh"
scratch="$(mktemp -d)"
source_tree="$scratch/source"
cleanup() {
    git -C "$root" worktree remove --force "$source_tree" >/dev/null 2>&1 || true
    rm -rf "$scratch"
}
trap cleanup EXIT
# This checkout supplies source files only. Its release scripts never execute.
git -C "$root" worktree add --quiet --detach "$source_tree" "$commit"
"$root/scripts/check-registry-config.py" --source-root "$source_tree"
"$root/scripts/release-crates.py" --source-root "$source_tree" check
version="$(cat "$source_tree/VERSION")"
[[ "$(release_tag)" == "v$version" ]] || {
    echo 'error: verified source version differs from ceremony tag' >&2
    exit 1
}
[[ "$(git -C "$source_tree" rev-parse HEAD)" == "$commit" && -z "$(git -C "$source_tree" status --porcelain --untracked-files=all)" ]]
while IFS= read -r predecessor; do
    [[ "$predecessor" != "$crate" ]] || break
    (
        cd "$source_tree"
        cargo +1.88.0 info --registry crates-io "$predecessor@$version" >/dev/null
    )
done <"$root/config/release/publishable-crates.txt"
(
    cd "$source_tree"
    cargo +1.88.0 publish --registry crates-io --dry-run --locked -p "$crate"
)
# The same original identity is required even across separately cued invocations.
WAITPRIMS_EXPECTED_TAG_OBJECT="$object" WAITPRIMS_EXPECTED_COMMIT="$commit" "$root/scripts/release-verify-published-tag.sh"
if [[ "$mode" == publish ]]; then
    "$root/scripts/check-registry-config.py" --source-root "$source_tree"
    [[ -z "$(git -C "$source_tree" status --porcelain --untracked-files=all)" ]] || {
        echo 'error: verified source changed during dry run' >&2
        exit 1
    }
    (
        cd "$source_tree"
        cargo +1.88.0 publish --registry crates-io --locked -p "$crate"
    )
    (
        cd "$source_tree"
        cargo +1.88.0 info --registry crates-io "$crate@$version"
    )
fi
