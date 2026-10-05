#!/usr/bin/env bash
# Committed public blobs remain authoritative despite operator-tree edits.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd -P)"
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
fixture="$scratch/repo"
git init -q -b main "$fixture"
git -C "$fixture" config user.name fixture
git -C "$fixture" config user.email fixture@example.invalid
mkdir -p "$fixture/scripts" "$fixture/keys" "$fixture/docs/security" "$fixture/docs/releases" "$scratch/assets"
cp "$root/scripts/"{release-common.sh,verify-staged-public.sh} "$fixture/scripts/"
for ext in txt ndjson; do
    printf 'public %s\n' "$ext" >"$fixture/keys/expected-fingerprints.$ext"
    cp "$fixture/keys/expected-fingerprints.$ext" "$scratch/assets/"
done
printf 'public gpg fixture\n' >"$fixture/docs/security/release-signing-keys.asc"
cp "$fixture/docs/security/release-signing-keys.asc" "$scratch/assets/waitprims-release-signing-key.asc"
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
./scripts/verify-staged-public.sh "$scratch/assets"
printf 'local tamper\n' >keys/expected-fingerprints.txt
rm docs/security/release-signing-keys.asc
./scripts/verify-staged-public.sh "$scratch/assets"
for name in expected-fingerprints.txt expected-fingerprints.ndjson waitprims-release-signing-key.asc release-notes-v1.2.3.md; do
    cp "$scratch/assets/$name" "$scratch/good"
    printf 'tampered staging\n' >"$scratch/assets/$name"
    if ./scripts/verify-staged-public.sh "$scratch/assets" >/dev/null 2>&1; then
        echo 'error: staged tamper accepted' >&2
        exit 1
    fi
    cp "$scratch/good" "$scratch/assets/$name"
done
echo '[ok] staged public material is bound to inert committed blobs'
