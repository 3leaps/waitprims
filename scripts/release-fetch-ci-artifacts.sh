#!/usr/bin/env bash
# Adapted from the spanwit exact-run handoff for five native waitprims targets.
set -euo pipefail
die() {
    echo "error: $*" >&2
    exit 1
}
root="$(cd "$(dirname "$0")/.." && pwd -P)"
cd "$root"
source "$root/scripts/release-common.sh"
export WAITPRIMS_RELEASE_TAG="${1:?canonical tag required}"
tag="$(release_tag)"
dest="${2:-dist/release}"
[[ ! -L "$dest" && ! -L "$dest.anchor" ]] || die 'unsafe artifact destination'
mkdir -p "$dest"
[[ -z "$(find "$dest" -mindepth 1 -maxdepth 1 -print)" ]] || die 'destination must be empty; clean explicitly first'
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
WAITPRIMS_ANCHOR_OUT="$scratch/anchor" "$root/scripts/release-verify-published-tag.sh"
commit="$(awk -F= '$1=="commit" {print $2}' "$scratch/anchor")"
object="$(awk -F= '$1=="object" {print $2}' "$scratch/anchor")"
[[ "$commit" =~ ^[0-9a-f]{40}$ && "$object" =~ ^[0-9a-f]{40}$ ]] || die 'verification returned no valid anchor'
runs="$(gh run list --repo "$WAITPRIMS_REPOSITORY" --workflow release.yml --branch "$tag" --limit 100 \
    --json databaseId,headSha,conclusion,event \
    --jq "[.[] | select(.headSha == \"$commit\" and .conclusion == \"success\" and .event == \"push\") | .databaseId]")"
run="${WAITPRIMS_RELEASE_RUN_ID:-}"
if [[ -n "$run" ]]; then
    [[ "$run" =~ ^[1-9][0-9]*$ ]] || die 'invalid selected run'
    jq -e --argjson run "$run" 'index($run) != null' <<<"$runs" >/dev/null || die 'selected run is not successful for this tag and commit'
else
    [[ "$(jq length <<<"$runs")" == 1 ]] || die 'select an exact successful run with WAITPRIMS_RELEASE_RUN_ID'
    run="$(jq -r '.[0]' <<<"$runs")"
fi
gh api "repos/$WAITPRIMS_REPOSITORY/actions/runs/$run" >"$scratch/run.json"
attempt="$(jq -r .run_attempt "$scratch/run.json")"
[[ "$attempt" =~ ^[1-9][0-9]*$ ]] || die 'invalid run attempt'
[[ -z "${WAITPRIMS_RELEASE_RUN_ATTEMPT:-}" || "$WAITPRIMS_RELEASE_RUN_ATTEMPT" == "$attempt" ]] || die 'requested attempt is not the current downloadable attempt'
jq -e --arg commit "$commit" --arg tag "$tag" \
    '.status == "completed" and .conclusion == "success" and .event == "push" and .head_sha == $commit and .head_branch == $tag and .path == ".github/workflows/release.yml"' "$scratch/run.json" >/dev/null || die 'workflow identity mismatch'
for suffix in linux-amd64 linux-arm64 darwin-arm64 windows-amd64 windows-arm64; do
    gh run download "$run" --repo "$WAITPRIMS_REPOSITORY" --name "cli-$suffix" --dir "$scratch/download/cli-$suffix"
done
gh run download "$run" --repo "$WAITPRIMS_REPOSITORY" --name sbom --dir "$scratch/download/sbom"
gh run download "$run" --repo "$WAITPRIMS_REPOSITORY" --name release-identity --dir "$scratch/download/identity"
# A successful rerun during download must not silently change the attempt.
[[ "$(gh api "repos/$WAITPRIMS_REPOSITORY/actions/runs/$run" --jq .run_attempt)" == "$attempt" ]] || die 'run attempt changed during download'
jq -e --arg tag "$tag" --arg object "$object" --arg commit "$commit" \
    '.tag == $tag and .object == $object and .commit == $commit' "$scratch/download/identity/release-identity.json" >/dev/null || die 'artifact tag object or commit mismatch'
[[ "$(find "$scratch/download" -name release-identity.json -type f | wc -l | tr -d ' ')" == 1 ]] || die 'duplicate or missing artifact identity'
while IFS= read -r -d '' file; do
    [[ -f "$file" && ! -L "$file" ]] || die 'unsafe artifact entry'
    name="${file##*/}"
    [[ "$name" != release-identity.json ]] || continue
    [[ ! -e "$dest/$name" ]] || die 'duplicate artifact basename'
    cp "$file" "$dest/$name"
done < <(find "$scratch/download" -mindepth 1 ! -type d -print0)
"$root/scripts/validate-release-assets.sh" "$dest" base
cp "$scratch/anchor" "$dest.anchor"
printf 'run=%s\nattempt=%s\n' "$run" "$attempt" >>"$dest.anchor"
require_published_anchor "$dest"
echo "[ok] staged exact tag object and commit from run $run attempt $attempt"
