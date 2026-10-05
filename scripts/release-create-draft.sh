#!/usr/bin/env bash
# Maintainer-only draft creation from the complete checksummed release set.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd -P)"
cd "$root"
source "$root/scripts/release-common.sh"
export WAITPRIMS_RELEASE_TAG="${1:?canonical tag required}"
directory="${2:-dist/release}"
tag="$(release_tag)"
"$root/scripts/validate-release-assets.sh" "$directory" checksummed
"$root/scripts/verify-checksums.sh" "$directory"
"$root/scripts/verify-staged-public.sh" "$directory"
# A 404 is the only absence proof. Transport/auth failures are not absence.
if gh api "repos/$WAITPRIMS_REPOSITORY/releases/tags/$tag" >/dev/null 2>"$directory.draft-error"; then
    echo 'error: release already exists' >&2
    exit 1
fi
grep -q 'HTTP 404' "$directory.draft-error" || {
    echo 'error: release absence unknown' >&2
    exit 1
}
rm -f "$directory.draft-error"
files=()
while IFS= read -r name; do files+=("$directory/$name"); done < <(release_checksummed_assets)
require_published_anchor "$directory"
commit="$(awk -F= '$1=="commit" {print $2}' "$directory.anchor")"
gh release create "$tag" --repo "$WAITPRIMS_REPOSITORY" --verify-tag --draft --target "$commit" \
    --title "$tag" --notes-file "$directory/release-notes-$tag.md" "${files[@]}"
