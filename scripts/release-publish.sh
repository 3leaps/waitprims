#!/usr/bin/env bash
# Maintainer promotion of a verified, exact draft release.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd -P)"
cd "$root"
source "$root/scripts/release-common.sh"
directory="${1:-dist/release}"
"$root/scripts/validate-release-assets.sh" "$directory" signed
"$root/scripts/verify-checksums.sh" "$directory"
"$root/scripts/verify-staged-public.sh" "$directory"
"$root/scripts/verify-signatures.sh" "$directory"
verify_remote_release_bytes "$directory"
require_published_anchor "$directory"
gh release edit "$(release_tag)" --repo "$WAITPRIMS_REPOSITORY" --draft=false
