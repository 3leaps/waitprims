#!/usr/bin/env bash
# Upload only the exact verified signed set; remain a draft.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd -P)"
cd "$root"
source "$root/scripts/release-common.sh"
export WAITPRIMS_RELEASE_TAG="${1:?canonical tag required}"
directory="${2:-dist/release}"
"$root/scripts/validate-release-assets.sh" "$directory" signed
"$root/scripts/verify-checksums.sh" "$directory"
"$root/scripts/verify-staged-public.sh" "$directory"
"$root/scripts/verify-signatures.sh" "$directory"
assert_github_release_state release_signed_assets resume
files=()
while IFS= read -r name; do files+=("$directory/$name"); done < <(release_signed_assets)
require_published_anchor "$directory"
gh release upload "$(release_tag)" --repo "$WAITPRIMS_REPOSITORY" "${files[@]}" --clobber
require_published_anchor "$directory"
gh release edit "$(release_tag)" --repo "$WAITPRIMS_REPOSITORY" --notes-file "$directory/release-notes-$(release_tag).md"
