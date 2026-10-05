#!/usr/bin/env bash
# Trusted newer guards, poisoned tagged scripts and cross-operation continuity.
# shellcheck disable=SC2016 # Tagged poison expands only if mistakenly executed.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd -P)"
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
fixture="$scratch/repo"
git init -q -b main "$fixture"
git -C "$fixture" config user.name fixture
git -C "$fixture" config user.email fixture@example.invalid
mkdir -p "$fixture/scripts" "$fixture/config/release" "$scratch/bin"
cp "$root/config/release/publishable-crates.txt" "$fixture/config/release/"
printf '[workspace]\nmembers=["crates/*"]\n' >"$fixture/Cargo.toml"
printf '1.2.3\n' >"$fixture/VERSION"
for suffix in core async testkit fs cli; do
    mkdir -p "$fixture/crates/waitprims-$suffix/src"
    publish=true
    [[ "$suffix" != cli ]] || publish=false
    printf '[package]\nname="waitprims-%s"\nversion="1.2.3"\npublish=%s\n' "$suffix" "$publish" >"$fixture/crates/waitprims-$suffix/Cargo.toml"
    printf '// synthetic source\n' >"$fixture/crates/waitprims-$suffix/src/lib.rs"
done
for name in release-common.sh release-crates-dry-run.sh release-crates-publish.sh release-crates.py check-registry-config.py release-verify-published-tag.sh; do
    printf '#!/usr/bin/env bash\ntouch "$FIXTURE_POISON"\nexit 0\n' >"$fixture/scripts/$name"
    chmod +x "$fixture/scripts/$name"
done
git -C "$fixture" add -A
git -C "$fixture" commit -qm 'synthetic release source with poisoned guards'
git -C "$fixture" tag -a v1.2.3 -m fixture
commit="$(git -C "$fixture" rev-parse HEAD)"
object="$(git -C "$fixture" rev-parse refs/tags/v1.2.3)"
printf 'tag=v1.2.3\nobject=%s\ncommit=%s\n' "$object" "$commit" >"$scratch/ceremony.anchor"
# Advance operator HEAD and install trusted guard code only in that checkout.
cp "$root/scripts/"{release-common.sh,release-crates-dry-run.sh,release-crates-publish.sh,release-crates.py,check-registry-config.py,release-record-anchor.sh} "$fixture/scripts/"
cat >"$fixture/scripts/release-verify-published-tag.sh" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
[[ "$WAITPRIMS_EXPECTED_TAG_OBJECT" == "$(git rev-parse refs/tags/v1.2.3)" && "$WAITPRIMS_EXPECTED_COMMIT" == "$FIXTURE_COMMIT" ]]
if [[ -n "${WAITPRIMS_ANCHOR_OUT:-}" ]]; then
 printf 'tag=v1.2.3\nobject=%s\ncommit=%s\n' "$WAITPRIMS_EXPECTED_TAG_OBJECT" "$WAITPRIMS_EXPECTED_COMMIT" > "$WAITPRIMS_ANCHOR_OUT"
fi
SH
chmod +x "$fixture/scripts/release-verify-published-tag.sh"
printf '9.0.0\n' >"$fixture/VERSION"
git -C "$fixture" add -A
git -C "$fixture" commit -qm 'newer trusted operator guards'
cat >"$scratch/bin/cargo" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
printf '%s | %s\n' "$PWD" "$*" >> "$FIXTURE_CALLS"
[[ "$(git rev-parse HEAD)" == "$FIXTURE_COMMIT" ]] || { echo 'error: Cargo did not use the recorded release source' >&2; exit 1; }
if [[ "$1" == metadata ]]; then
 python3 - <<'PY'
import json,pathlib,tomllib
root=pathlib.Path.cwd()
dependencies={'core':[],'async':['core'],'testkit':['core','async'],'fs':['core','async','testkit'],'cli':['core']}
packages=[]
for suffix in dependencies:
    manifest=root/'crates'/('waitprims-'+suffix)/'Cargo.toml'
    data=tomllib.loads(manifest.read_text())['package']
    packages.append({'name':data['name'],'publish':None if data['publish'] else [],'manifest_path':str(manifest),'dependencies':[{'name':'waitprims-'+dep} for dep in dependencies[suffix]]})
print(json.dumps({'packages':packages}))
PY
elif [[ "$*" == *publish* && "$*" != *--dry-run* ]]; then
 printf upload >> "$FIXTURE_UPLOAD"
fi
SH
chmod +x "$scratch/bin/cargo"
export PATH="$scratch/bin:$PATH" FIXTURE_COMMIT="$commit" FIXTURE_POISON="$scratch/poison" FIXTURE_CALLS="$scratch/calls" FIXTURE_UPLOAD="$scratch/upload"
export WAITPRIMS_RELEASE_TAG=v1.2.3 WAITPRIMS_RELEASE_ANCHOR_FILE="$scratch/ceremony.anchor"
"$fixture/scripts/release-crates-dry-run.sh" waitprims-fs
[[ ! -e "$scratch/poison" && ! -e "$scratch/upload" ]]
grep -q 'info --registry crates-io waitprims-core@1.2.3' "$scratch/calls"
grep -q 'info --registry crates-io waitprims-async@1.2.3' "$scratch/calls"
grep -q 'info --registry crates-io waitprims-testkit@1.2.3' "$scratch/calls"
"$fixture/scripts/release-crates-publish.sh" waitprims-fs
[[ "$(cat "$scratch/upload")" == upload && ! -e "$scratch/poison" ]]
fail() { if "$@" >"$scratch/output" 2>&1; then
    echo 'error: registry boundary rejection missing' >&2
    exit 1
fi; }
env WAITPRIMS_RELEASE_ANCHOR_FILE="$scratch/recorded.anchor" WAITPRIMS_EXPECTED_TAG_OBJECT="$object" WAITPRIMS_EXPECTED_COMMIT="$commit" "$fixture/scripts/release-record-anchor.sh"
cmp "$scratch/ceremony.anchor" "$scratch/recorded.anchor"
fail env WAITPRIMS_RELEASE_ANCHOR_FILE="$scratch/recorded.anchor" WAITPRIMS_EXPECTED_TAG_OBJECT="$object" WAITPRIMS_EXPECTED_COMMIT="$commit" "$fixture/scripts/release-record-anchor.sh"
fail env -u WAITPRIMS_RELEASE_ANCHOR_FILE "$fixture/scripts/release-crates-dry-run.sh" waitprims-core
# Same commit, different annotated object: a new operation must retain the original.
git -C "$fixture" tag -fa v1.2.3 "$commit" -m changed >/dev/null
rm "$scratch/upload"
fail "$fixture/scripts/release-crates-dry-run.sh" waitprims-core
fail "$fixture/scripts/release-crates-publish.sh" waitprims-core
[[ ! -e "$scratch/upload" && ! -e "$scratch/poison" ]]
[[ "$(git -C "$fixture" worktree list --porcelain | grep -c '^worktree ')" == 1 ]]
echo '[ok] newer trusted registry guards, inert tagged scripts and original-identity continuity'
