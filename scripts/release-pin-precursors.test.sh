#!/usr/bin/env bash
# Adapted public-pin precursor controls; only temporary synthetic signing homes.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd -P)"
fixture="$(mktemp -d)"
cleanup() {
    for home in "$fixture/home" "$fixture/default/.gnupg" "$fixture/repo/in-repo-home"; do
        [[ ! -d "$home" ]] || gpgconf --homedir "$home" --kill all >/dev/null 2>&1 || true
    done
    rm -rf "$fixture"
}
trap cleanup EXIT
mkdir -p "$fixture/repo/scripts" "$fixture/repo/docs" "$fixture/home" "$fixture/default/.gnupg"
chmod 700 "$fixture/home" "$fixture/default/.gnupg"
cp "$root/scripts/"{release-export-pin.sh,release-validate-pin.sh,release-decernor.sh} "$fixture/repo/scripts/"
export WAITPRIMS_DECERNOR_BIN="${WAITPRIMS_DECERNOR_BIN:-$(command -v decernor)}"
python3 - "$fixture/public.pub" "$fixture/private.pub" <<'PY'
import base64
import pathlib
import sys
pathlib.Path(sys.argv[1]).write_text('untrusted comment: synthetic public key\n' + base64.b64encode(b'Ed' + b'\x01' * 40).decode() + '\n')
pathlib.Path(sys.argv[2]).write_text('untrusted comment: minisign encrypted secret key\n' + base64.b64encode(b'\x02' * 128).decode() + '\n')
PY
gpg --homedir "$fixture/home" --batch --pinentry-mode loopback --passphrase '' \
    --quick-gen-key 'Synthetic signing <signing@example.invalid>' ed25519 cert 1d >/dev/null 2>&1
primary="$(gpg --homedir "$fixture/home" --batch --with-colons --fingerprint --list-keys | awk -F: '$1=="fpr" {print $10;exit}')"
gpg --homedir "$fixture/home" --batch --pinentry-mode loopback --passphrase '' \
    --quick-add-key "$primary" ed25519 sign 1d >/dev/null 2>&1
selector="$(gpg --homedir "$fixture/home" --batch --with-colons --with-subkey-fingerprint --list-keys | awk -F: '$1=="sub" {s=1;next} s && $1=="fpr" {print $10 "!";exit}')"
export WAITPRIMS_GPG_HOMEDIR="$fixture/home" WAITPRIMS_GPG_SIGNING_FINGERPRINT="$primary" WAITPRIMS_PGP_KEY_ID="$selector" WAITPRIMS_MINISIGN_PUB="$fixture/public.pub"
pin="$fixture/repo/docs/security/release-signing-keys.asc"
exporter="$fixture/repo/scripts/release-export-pin.sh"
validator="$fixture/repo/scripts/release-validate-pin.sh"
fail() {
    if "$@" >"$fixture/output" 2>&1; then
        echo 'error: expected pin precursor rejection' >&2
        exit 1
    fi
}
fail "$validator"
fail env WAITPRIMS_MINISIGN_PUB="$fixture/missing" "$exporter"
[[ ! -e "$pin" ]]
mkdir -m 700 "$fixture/repo/in-repo-home"
fail env WAITPRIMS_GPG_HOMEDIR="$fixture/repo/in-repo-home" "$exporter"
[[ ! -e "$pin" ]]
fail env HOME="$fixture/default" WAITPRIMS_GPG_HOMEDIR="$fixture/default/.gnupg" "$exporter"
[[ ! -e "$pin" ]]
"$exporter" >"$fixture/output"
cp "$pin" "$fixture/good.asc"
"$validator" >"$fixture/output"
fail "$exporter"
cmp "$pin" "$fixture/good.asc"
fail env WAITPRIMS_MINISIGN_PUB="$fixture/private.pub" "$validator"
fail env WAITPRIMS_GPG_SIGNING_FINGERPRINT=0000000000000000000000000000000000000000 "$validator"
fail env WAITPRIMS_PGP_KEY_ID=0000000000000000000000000000000000000000! "$validator"
fail env WAITPRIMS_PGP_KEY_ID="${selector%!}" "$validator"
gpg --homedir "$fixture/home" --batch --armor --export-secret-keys "$selector" >"$pin"
fail "$validator"
cp "$fixture/good.asc" "$pin"
gpg --homedir "$fixture/home" --batch --armor --export-secret-keys "$selector" >"$fixture/repo/docs/security/unexpected.asc"
fail "$validator"
rm "$fixture/repo/docs/security/unexpected.asc"
rm "$pin"
ln -s "$fixture/good.asc" "$pin"
fail "$exporter"
fail "$validator"
echo '[ok] public pin identity, no-overwrite, secret-material and external-home controls'
