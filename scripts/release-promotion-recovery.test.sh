#!/usr/bin/env bash
# Remote bytes and partial/completed retry controls, without real API writes.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd -P)"
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
fixture="$scratch/repo"
git init -q -b main "$fixture"
git -C "$fixture" config user.name fixture
git -C "$fixture" config user.email fixture@example.invalid
mkdir -p "$fixture/scripts" "$scratch/local" "$scratch/remote" "$scratch/bin"
cp "$root/scripts/"{release-common.sh,release-publish.sh,upload-release-assets.sh,validate-release-assets.sh,verify-checksums.sh} "$fixture/scripts/"
for helper in verify-staged-public verify-signatures; do printf '#!/usr/bin/env bash\nexit 0\n' >"$fixture/scripts/$helper.sh"; done
cat >"$fixture/scripts/release-verify-published-tag.sh" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
[[ "${MOVED:-0}" == 0 && "$WAITPRIMS_EXPECTED_COMMIT" == "$(git rev-parse refs/tags/v1.2.3^{})" ]]
SH
chmod +x "$fixture/scripts/"*.sh
git -C "$fixture" add -A
git -C "$fixture" commit -qm fixture
git -C "$fixture" tag v1.2.3
export WAITPRIMS_RELEASE_TAG=v1.2.3
source "$root/scripts/release-common.sh"
while IFS= read -r name; do printf 'synthetic %s\n' "$name" >"$scratch/local/$name"; done < <(release_signable_assets)
files=()
while IFS= read -r name; do files+=("$name"); done < <(release_signable_assets | LC_ALL=C sort)
(
    cd "$scratch/local"
    shasum -a 256 "${files[@]}" >SHA256SUMS
    shasum -a 512 "${files[@]}" >SHA512SUMS
)
printf synthetic >"$scratch/local/SHA256SUMS.minisig"
printf synthetic >"$scratch/local/SHA512SUMS.minisig"
cp "$scratch/local/"* "$scratch/remote/"
commit="$(git -C "$fixture" rev-parse HEAD)"
printf 'tag=v1.2.3\nobject=%040d\ncommit=%s\n' 1 "$commit" >"$scratch/local.anchor"
cat >"$scratch/bin/gh" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> "$FIXTURE_CALLS"
case "$1 $2" in
 'repo view') echo 3leaps/waitprims ;;
 'release view')
  python3 - "$FIXTURE_REMOTE" "$FIXTURE_COMMIT" <<'PY'
import json,os,pathlib,sys
print(json.dumps({'tagName':'v1.2.3','targetCommitish':os.environ.get('BAD_TARGET',sys.argv[2]),'isDraft':os.environ.get('PUBLIC','0')!='1','assets':[{'name':p.name} for p in pathlib.Path(sys.argv[1]).iterdir()]}))
PY
 ;;
 'release download')
  while [[ $# -gt 0 ]]; do [[ "$1" != --dir ]] || { destination="$2"; break; }; shift; done
  cp "$FIXTURE_REMOTE/"* "$destination/" ;;
 'release upload')
  for file in "$@"; do [[ ! -f "$file" ]] || cp "$file" "$FIXTURE_REMOTE/"; done ;;
 'release edit') echo edited ;;
 *) exit 1 ;;
esac
SH
chmod +x "$scratch/bin/gh"
export PATH="$scratch/bin:$PATH" FIXTURE_REMOTE="$scratch/remote" FIXTURE_COMMIT="$commit" FIXTURE_CALLS="$scratch/calls"
cd "$fixture"
fail() {
    : >"$scratch/calls"
    if "$@" >"$scratch/output" 2>&1; then
        echo 'error: expected promotion/recovery rejection' >&2
        exit 1
    fi
    if grep -q -- '--draft=false' "$scratch/calls"; then
        echo 'error: held bytes were promoted' >&2
        exit 1
    fi
}
./scripts/release-publish.sh "$scratch/local" >/dev/null
for name in waitprims-1.2.3-linux-amd64.tar.gz SHA256SUMS SHA256SUMS.minisig; do
    printf poison >"$scratch/remote/$name"
    fail ./scripts/release-publish.sh "$scratch/local"
    cp "$scratch/local/$name" "$scratch/remote/$name"
done
# Known draft subsets and complete signed drafts may be safely restored.
rm "$scratch/remote/SHA256SUMS.minisig" "$scratch/remote/SHA512SUMS.minisig"
./scripts/upload-release-assets.sh v1.2.3 "$scratch/local" >/dev/null
rm "$scratch/remote/SHA512SUMS.minisig"
printf poison >"$scratch/remote/SHA256SUMS.minisig"
./scripts/upload-release-assets.sh v1.2.3 "$scratch/local" >/dev/null
./scripts/upload-release-assets.sh v1.2.3 "$scratch/local" >/dev/null
./scripts/release-publish.sh "$scratch/local" >/dev/null
printf extra >"$scratch/remote/extra"
fail ./scripts/upload-release-assets.sh v1.2.3 "$scratch/local"
rm "$scratch/remote/extra"
fail env BAD_TARGET=wrong ./scripts/upload-release-assets.sh v1.2.3 "$scratch/local"
fail env PUBLIC=1 ./scripts/upload-release-assets.sh v1.2.3 "$scratch/local"
fail env MOVED=1 ./scripts/upload-release-assets.sh v1.2.3 "$scratch/local"
echo '[ok] remote byte agreement and safe partial/completed draft-upload retries'
