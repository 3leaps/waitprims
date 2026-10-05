#!/usr/bin/env bash
# Exact inventories and manifests; synthetic bytes only.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd -P)"
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
export WAITPRIMS_RELEASE_TAG=v1.2.3
source "$root/scripts/release-common.sh"
directory="$scratch/assets"
mkdir "$directory"
while IFS= read -r name; do printf 'synthetic %s\n' "$name" >"$directory/$name"; done < <(release_signable_assets)
files=()
while IFS= read -r name; do files+=("$name"); done < <(release_signable_assets | LC_ALL=C sort)
(
    cd "$directory"
    shasum -a 256 "${files[@]}" >SHA256SUMS
    shasum -a 512 "${files[@]}" >SHA512SUMS
)
"$root/scripts/validate-release-assets.sh" "$directory" checksummed
"$root/scripts/verify-checksums.sh" "$directory"
fail() { if "$@" >"$scratch/output" 2>&1; then
    echo 'error: expected asset rejection' >&2
    exit 1
fi; }
cp "$directory/SHA256SUMS" "$scratch/good256"
cp "$directory/SHA512SUMS" "$scratch/good512"
printf extra >"$directory/.hidden"
fail "$root/scripts/validate-release-assets.sh" "$directory" checksummed
rm "$directory/.hidden"
mv "$directory/waitprims-1.2.3-windows-arm64.zip" "$scratch/asset"
fail "$root/scripts/validate-release-assets.sh" "$directory" checksummed
fail "$root/scripts/verify-checksums.sh" "$directory"
mv "$scratch/asset" "$directory/waitprims-1.2.3-windows-arm64.zip"
cp "$directory/waitprims-1.2.3-windows-arm64.zip" "$directory/waitprims-1.2.4-windows-arm64.zip"
fail "$root/scripts/validate-release-assets.sh" "$directory" checksummed
rm "$directory/waitprims-1.2.4-windows-arm64.zip"
printf changed >>"$directory/expected-fingerprints.ndjson"
fail "$root/scripts/verify-checksums.sh" "$directory"
printf 'synthetic expected-fingerprints.ndjson\n' >"$directory/expected-fingerprints.ndjson"
cat "$scratch/good256" >>"$directory/SHA256SUMS"
fail "$root/scripts/verify-checksums.sh" "$directory"
cp "$scratch/good256" "$directory/SHA256SUMS"
rm "$directory/SHA512SUMS"
fail "$root/scripts/verify-checksums.sh" "$directory"
cp "$scratch/good512" "$directory/SHA512SUMS"
mv "$directory/LICENSE-MIT" "$scratch/license"
ln -s "$scratch/license" "$directory/LICENSE-MIT"
fail "$root/scripts/validate-release-assets.sh" "$directory" checksummed
fail "$root/scripts/verify-checksums.sh" "$directory"
rm "$directory/LICENSE-MIT"
mv "$scratch/license" "$directory/LICENSE-MIT"
# Absent manifests cannot produce a vacuous signature pass.
mkdir "$scratch/empty"
fail "$root/scripts/verify-signatures.sh" "$scratch/empty"
echo '[ok] exact release inventory and checksum negative controls'
