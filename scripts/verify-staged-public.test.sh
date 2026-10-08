#!/usr/bin/env bash
# Committed public blobs remain authoritative despite operator-tree edits.
# Cryptographic checks are covered by the Decernor and post-tag selector suites.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd -P)"
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
fixture="$scratch/repo"
git init -q -b main "$fixture"
git -C "$fixture" config user.name fixture
git -C "$fixture" config user.email fixture@example.invalid
mkdir -p "$fixture/scripts" "$fixture/keys" "$fixture/docs/security" "$fixture/docs/releases" "$scratch/assets"
cp "$root/scripts/"{release-common.sh,release-stage-public.sh,verify-staged-public.sh} "$fixture/scripts/"
for ext in txt ndjson; do
    printf 'public %s\n' "$ext" >"$fixture/keys/expected-fingerprints.$ext"
    cp "$fixture/keys/expected-fingerprints.$ext" "$scratch/assets/"
done
printf 'public gpg fixture\n' >"$fixture/docs/security/release-signing-keys.asc"
cp "$fixture/docs/security/release-signing-keys.asc" "$scratch/assets/waitprims-release-signing-key.asc"
python3 - "$fixture/docs/security/waitprims-minisign.pub" <<'PY'
import base64
import pathlib
import sys
pathlib.Path(sys.argv[1]).write_text('untrusted comment: synthetic public key\n' + base64.b64encode(b'Ed' + b'\x01' * 40).decode() + '\n')
PY
cp "$fixture/docs/security/waitprims-minisign.pub" "$scratch/minisign.good"
printf 'cut notes\n' >"$fixture/docs/releases/v1.2.3.md"
cp "$fixture/docs/releases/v1.2.3.md" "$scratch/assets/release-notes-v1.2.3.md"
printf '#!/usr/bin/env bash\nexit 0\n' >"$fixture/scripts/verify-public-keys.sh"
cat >"$fixture/scripts/release-verify-published-tag.sh" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
[[ "$WAITPRIMS_EXPECTED_COMMIT" == "$(git rev-parse refs/tags/v1.2.3^{})" ]]
SH
chmod +x "$fixture/scripts/"*.sh
git -C "$fixture" add -A
git -C "$fixture" commit -qm fixture
git -C "$fixture" tag v1.2.3
printf 'tag=v1.2.3\nobject=%040d\ncommit=%s\n' 1 "$(git -C "$fixture" rev-parse HEAD)" >"$scratch/assets.anchor"
export WAITPRIMS_RELEASE_TAG=v1.2.3
cd "$fixture"
fail() {
    if "$@" >"$scratch/output" 2>&1; then
        echo 'error: expected committed public material rejection' >&2
        exit 1
    fi
}
# No external public export is required, and an unrelated input is ignored.
unset WAITPRIMS_MINISIGN_PUB
./scripts/release-stage-public.sh "$scratch/assets"
cmp "$scratch/minisign.good" "$scratch/assets/waitprims-minisign.pub"
export WAITPRIMS_MINISIGN_PUB="$scratch/missing-external.pub"
./scripts/release-stage-public.sh "$scratch/assets"
./scripts/verify-staged-public.sh "$scratch/assets"
printf 'local tamper\n' >keys/expected-fingerprints.txt
rm docs/security/release-signing-keys.asc
# A newer committed operator key must not replace the approved old tag's key.
printf 'newer public key\n' >docs/security/waitprims-minisign.pub
git add -A
git commit -qm 'newer operator state'
./scripts/release-stage-public.sh "$scratch/assets"
cmp "$scratch/minisign.good" "$scratch/assets/waitprims-minisign.pub"
./scripts/verify-staged-public.sh "$scratch/assets"
for name in expected-fingerprints.txt expected-fingerprints.ndjson waitprims-release-signing-key.asc waitprims-minisign.pub release-notes-v1.2.3.md; do
    cp "$scratch/assets/$name" "$scratch/good"
    printf 'tampered staging\n' >"$scratch/assets/$name"
    if ./scripts/verify-staged-public.sh "$scratch/assets" >/dev/null 2>&1; then
        echo 'error: staged tamper accepted' >&2
        exit 1
    fi
    cp "$scratch/good" "$scratch/assets/$name"
done
# Blob-equivalent minisign presentation changes still fail byte binding.
sed '1s/.*/untrusted comment: changed presentation/' "$scratch/minisign.good" >"$scratch/assets/waitprims-minisign.pub"
fail ./scripts/verify-staged-public.sh "$scratch/assets"
cp "$scratch/minisign.good" "$scratch/assets/waitprims-minisign.pub"
rm "$scratch/assets/waitprims-minisign.pub"
fail ./scripts/verify-staged-public.sh "$scratch/assets"
ln -s "$scratch/minisign.good" "$scratch/assets/waitprims-minisign.pub"
fail ./scripts/verify-staged-public.sh "$scratch/assets"
fail ./scripts/release-stage-public.sh "$scratch/assets"
rm "$scratch/assets/waitprims-minisign.pub"
./scripts/release-stage-public.sh "$scratch/assets"

# Reject tags with a missing, symlink or empty public minisign blob.
cp "$scratch/assets/waitprims-release-signing-key.asc" docs/security/release-signing-keys.asc
cp "$scratch/assets/expected-fingerprints.txt" keys/expected-fingerprints.txt
rm docs/security/waitprims-minisign.pub
git add -A
git commit -qm 'missing public key fixture'
git tag -f v1.2.3 >/dev/null
printf 'tag=v1.2.3\nobject=%040d\ncommit=%s\n' 1 "$(git rev-parse HEAD)" >"$scratch/assets.anchor"
fail ./scripts/release-stage-public.sh "$scratch/assets"
fail ./scripts/verify-staged-public.sh "$scratch/assets"
ln -s "$scratch/minisign.good" docs/security/waitprims-minisign.pub
git add -A
git commit -qm 'symlink public key fixture'
git tag -f v1.2.3 >/dev/null
printf 'tag=v1.2.3\nobject=%040d\ncommit=%s\n' 1 "$(git rev-parse HEAD)" >"$scratch/assets.anchor"
fail ./scripts/release-stage-public.sh "$scratch/assets"
fail ./scripts/verify-staged-public.sh "$scratch/assets"
rm docs/security/waitprims-minisign.pub
: >docs/security/waitprims-minisign.pub
git add -A
git commit -qm 'empty public key fixture'
git tag -f v1.2.3 >/dev/null
printf 'tag=v1.2.3\nobject=%040d\ncommit=%s\n' 1 "$(git rev-parse HEAD)" >"$scratch/assets.anchor"
fail ./scripts/release-stage-public.sh "$scratch/assets"
: >"$scratch/assets/waitprims-minisign.pub"
fail ./scripts/verify-staged-public.sh "$scratch/assets"
echo '[ok] staged public material is bound to inert committed blobs, including minisign'
