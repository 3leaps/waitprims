#!/usr/bin/env bash
# Manifest signing selectors bind to the verified released public material.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd -P)"
cd "$root"
directory="${1:-dist/release}"
"$root/scripts/verify-staged-public.sh" "$directory"
approved="${WAITPRIMS_GPG_SIGNING_FINGERPRINT:-}"
selector="${WAITPRIMS_PGP_KEY_ID:-}"
[[ "$approved" =~ ^[0-9A-F]{40}$ && "$selector" =~ ^[0-9A-F]{40}!$ ]] || {
    echo 'error: approved primary and exact subkey required' >&2
    exit 1
}
[[ "$(awk '$1=="gpg" {print $2}' "$directory/expected-fingerprints.txt")" == "$approved" ]] || {
    echo 'error: released anchor differs from approved primary' >&2
    exit 1
}
home="${WAITPRIMS_GPG_HOMEDIR:-}"
[[ "$home" == /* && -d "$home" ]] || {
    echo 'error: external signing home required' >&2
    exit 1
}
home="$(cd "$home" && pwd -P)"
case "$home" in "$root" | "$root"/*)
    echo 'error: signing home must be outside trusted checkout' >&2
    exit 1
    ;;
esac
keyring="$(mktemp -d)"
trap 'gpgconf --homedir "$keyring" --kill all >/dev/null 2>&1 || true; rm -rf "$keyring"' EXIT
chmod 700 "$keyring"
pin_listing="$(gpg --homedir "$keyring" --batch --with-colons --with-subkey-fingerprint --show-keys "$directory/waitprims-release-signing-key.asc" 2>/dev/null)"
permitted="$(awk -F: '$1=="sub" {s=1;cap=$12;next} s && $1=="fpr" {if(cap ~ /s/) print $10; s=0}' <<<"$pin_listing")"
[[ "$permitted!" == "$selector" ]] || {
    echo 'error: selector differs from released signing subkey' >&2
    exit 1
}
listing="$(gpg --homedir "$home" --batch --with-colons --fingerprint --with-subkey-fingerprint --list-keys "${selector%!}" 2>/dev/null)"
primary="$(awk -F: '$1=="pub" {p=1;next} p && $1=="fpr" {print $10;exit}' <<<"$listing")"
subkey="$(awk -F: -v wanted="${selector%!}" '$1=="sub" {s=1;cap=$12;valid=$2;next} s && $1=="fpr" {if($10==wanted && cap ~ /s/ && valid !~ /[redi]/) print $10; s=0}' <<<"$listing")"
[[ "$primary" == "$approved" && "$subkey!" == "$selector" ]] || {
    echo 'error: configured home lacks the approved released signing subkey' >&2
    exit 1
}
echo '[ok] post-tag selector uses the approved released pin'
