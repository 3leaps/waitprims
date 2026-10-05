#!/usr/bin/env bash
# Mock remote/API state; cryptographic controls are tested separately.
# shellcheck disable=SC2016 # Positional parameters expand in the child bash.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd -P)"
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
fixture="$scratch/repo"
git init -q "$fixture"
mkdir -p "$fixture/scripts" "$scratch/bin"
cp "$root/scripts/"{release-common.sh,release-fetch-ci-artifacts.sh,release-create-draft.sh,release-publish.sh,validate-release-assets.sh,verify-checksums.sh} "$fixture/scripts/"
cat >"$fixture/scripts/release-verify-published-tag.sh" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
[[ "${MOVED:-0}" == 0 ]] || exit 1
object=1111111111111111111111111111111111111111
commit=2222222222222222222222222222222222222222
[[ -z "${WAITPRIMS_EXPECTED_TAG_OBJECT:-}" || "$WAITPRIMS_EXPECTED_TAG_OBJECT" == "$object" ]] || exit 1
[[ -z "${WAITPRIMS_EXPECTED_COMMIT:-}" || "$WAITPRIMS_EXPECTED_COMMIT" == "$commit" ]] || exit 1
if [[ -n "${WAITPRIMS_ANCHOR_OUT:-}" ]]; then
 printf 'tag=v1.2.3\nobject=%s\ncommit=%s\n' "$object" "$commit" > "$WAITPRIMS_ANCHOR_OUT"
fi
SH
for helper in verify-public-keys verify-staged-public verify-signatures; do
    printf '#!/usr/bin/env bash\nexit 0\n' >"$fixture/scripts/$helper.sh"
done
chmod +x "$fixture/scripts/"*.sh
cat >"$scratch/bin/gh" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> "$FIXTURE_CALLS"
case "$1 $2" in
 'run list')
  [[ "${MULTIPLE:-0}" == 0 ]] && echo '[7]' || echo '[7,8]' ;;
 'api repos/3leaps/waitprims/actions/runs/7')
  if [[ "$*" == *'--jq'* ]]; then
   [[ "${ATTEMPT_MOVED:-0}" == 0 ]] && echo 2 || echo 3
  else
   printf '{"run_attempt":2,"status":"completed","conclusion":"success","event":"push","head_sha":"2222222222222222222222222222222222222222","head_branch":"v1.2.3","path":".github/workflows/release.yml"}\n'
  fi ;;
 'run download')
  name= dir=
  while [[ $# -gt 0 ]]; do
   case "$1" in --name) name="$2"; shift;; --dir) dir="$2"; shift;; esac
   shift
  done
  mkdir -p "$dir"
  case "$name" in
   cli-*)
    suffix="${name#cli-}"; ext=tar.gz; [[ "$suffix" == windows-* ]] && ext=zip
    [[ "$suffix" != "${MISSING_SUFFIX:-}" ]] && printf fixture > "$dir/waitprims-1.2.3-$suffix.$ext" || true ;;
   sbom) printf fixture > "$dir/sbom-1.2.3.cdx.json" ;;
   release-identity)
    object=1111111111111111111111111111111111111111
    [[ "${BAD_OBJECT:-0}" == 0 ]] || object=3333333333333333333333333333333333333333
    printf '{"tag":"v1.2.3","object":"%s","commit":"2222222222222222222222222222222222222222"}\n' "$object" > "$dir/release-identity.json"
    printf fixture > "$dir/LICENSE-MIT"; printf fixture > "$dir/LICENSE-APACHE"
    [[ "${EXTRA:-0}" == 0 ]] || printf extra > "$dir/.extra" ;;
   *) exit 1 ;;
  esac ;;
 'api repos/3leaps/waitprims/releases/tags/v1.2.3')
  [[ "${RELEASE_EXISTS:-0}" == 0 ]] || { echo '{}'; exit 0; }
  [[ "${API_UNKNOWN:-0}" == 0 ]] && echo 'gh: Not Found (HTTP 404)' >&2 || echo 'gh: Forbidden (HTTP 403)' >&2
  exit 1 ;;
 'release create') echo created ;;
 *) exit 1 ;;
esac
SH
chmod +x "$scratch/bin/gh"
export PATH="$scratch/bin:$PATH" FIXTURE_CALLS="$scratch/calls"
export WAITPRIMS_RELEASE_TAG=v1.2.3
cd "$fixture"
fail() { if "$@" >"$scratch/output" 2>&1; then
    echo 'error: expected handoff rejection' >&2
    exit 1
fi; }
fetch() {
    rm -rf "$scratch/assets" "$scratch/assets.anchor"
    ./scripts/release-fetch-ci-artifacts.sh v1.2.3 "$scratch/assets"
}
fetch >/dev/null
fail env BAD_OBJECT=1 bash -c 'rm -rf "$1"; "$2" v1.2.3 "$1"' _ "$scratch/bad" "$fixture/scripts/release-fetch-ci-artifacts.sh"
fail env ATTEMPT_MOVED=1 bash -c 'rm -rf "$1"; "$2" v1.2.3 "$1"' _ "$scratch/bad" "$fixture/scripts/release-fetch-ci-artifacts.sh"
fail env MISSING_SUFFIX=windows-arm64 bash -c 'rm -rf "$1"; "$2" v1.2.3 "$1"' _ "$scratch/bad" "$fixture/scripts/release-fetch-ci-artifacts.sh"
fail env EXTRA=1 bash -c 'rm -rf "$1"; "$2" v1.2.3 "$1"' _ "$scratch/bad" "$fixture/scripts/release-fetch-ci-artifacts.sh"
fail env MULTIPLE=1 bash -c 'rm -rf "$1"; "$2" v1.2.3 "$1"' _ "$scratch/bad" "$fixture/scripts/release-fetch-ci-artifacts.sh"
fail env WAITPRIMS_RELEASE_RUN_ATTEMPT=1 bash -c 'rm -rf "$1"; "$2" v1.2.3 "$1"' _ "$scratch/bad" "$fixture/scripts/release-fetch-ci-artifacts.sh"
source ./scripts/release-common.sh
while IFS= read -r name; do [[ -f "$scratch/assets/$name" ]] || printf fixture >"$scratch/assets/$name"; done < <(release_signable_assets)
files=()
while IFS= read -r name; do files+=("$name"); done < <(release_signable_assets | LC_ALL=C sort)
(
    cd "$scratch/assets"
    shasum -a 256 "${files[@]}" >SHA256SUMS
    shasum -a 512 "${files[@]}" >SHA512SUMS
)
./scripts/release-create-draft.sh v1.2.3 "$scratch/assets" >/dev/null
fail env MOVED=1 ./scripts/release-create-draft.sh v1.2.3 "$scratch/assets"
fail env RELEASE_EXISTS=1 ./scripts/release-create-draft.sh v1.2.3 "$scratch/assets"
fail env API_UNKNOWN=1 ./scripts/release-create-draft.sh v1.2.3 "$scratch/assets"
sed 's/object=1111111111111111111111111111111111111111/object=3333333333333333333333333333333333333333/' "$scratch/assets.anchor" >"$scratch/changed"
cp "$scratch/changed" "$scratch/assets.anchor"
fail ./scripts/release-create-draft.sh v1.2.3 "$scratch/assets"
echo '[ok] CI artifact object/commit/attempt and local draft controls'
