#!/usr/bin/env bash
# Shared release asset and repository invariants.

set -euo pipefail

WAITPRIMS_REPOSITORY="3leaps/waitprims"

release_repo_root() {
    (cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)
}

release_version() {
    local repo_path
    repo_path="$(release_repo_root)"
    local version
    version="$(cat "$repo_path/VERSION")"
    if [[ ! "$version" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]]; then
        echo "error: VERSION must contain one stable semantic version" >&2
        return 1
    fi
    printf '%s\n' "$version"
}

release_tag() {
    local tag="${1:-${WAITPRIMS_RELEASE_TAG:-}}"
    if [[ -z "$tag" ]]; then
        echo "error: WAITPRIMS_RELEASE_TAG is required" >&2
        return 1
    fi
    if [[ ! "$tag" =~ ^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]]; then
        echo "error: release tag must be canonical vX.Y.Z" >&2
        return 1
    fi
    printf '%s\n' "$tag"
}

require_release_guard() {
    local strict="${1:-0}"
    local repo_path
    repo_path="$(release_repo_root)"
    if [[ "$strict" == "1" ]]; then
        WAITPRIMS_REQUIRE_TAG=1 "$repo_path/scripts/release-guard-tag-version.sh"
    else
        "$repo_path/scripts/release-guard-tag-version.sh"
    fi
}

release_base_assets() {
    local version
    version="$(release_tag)"
    version="${version#v}"
    printf '%s\n' LICENSE-APACHE LICENSE-MIT "sbom-${version}.cdx.json"
    local platform ext
    for platform in linux-amd64 linux-arm64 darwin-arm64 windows-amd64 windows-arm64; do
        ext=tar.gz
        [[ "$platform" == windows-* ]] && ext=zip
        printf 'waitprims-%s-%s.%s\n' "$version" "$platform" "$ext"
    done
}

release_signable_assets() {
    local tag
    tag="$(release_tag)"
    release_base_assets
    printf 'release-notes-%s.md\n' "$tag"
    printf '%s\n' expected-fingerprints.txt expected-fingerprints.ndjson waitprims-minisign.pub waitprims-release-signing-key.asc
}

release_provenance_assets() {
    printf '%s\n' \
        "SHA256SUMS" \
        "SHA256SUMS.minisig" \
        "SHA512SUMS" \
        "SHA512SUMS.minisig" \
        "waitprims-minisign.pub"
    if [[ -n "${WAITPRIMS_PGP_KEY_ID:-}" || -n "${WAITPRIMS_GPG_HOMEDIR:-}" ]]; then
        require_complete_pgp_config
        printf '%s\n' \
            "SHA256SUMS.asc" \
            "SHA512SUMS.asc" \
            "waitprims-release-signing-key.asc"
    fi
}

release_checksummed_assets() {
    release_signable_assets
    printf '%s\n' SHA256SUMS SHA512SUMS
}

release_signed_without_keys_assets() {
    release_checksummed_assets
    printf '%s\n' SHA256SUMS.minisig SHA512SUMS.minisig
    if [[ -n "${WAITPRIMS_PGP_KEY_ID:-}" || -n "${WAITPRIMS_GPG_HOMEDIR:-}" ]]; then
        require_complete_pgp_config
        printf '%s\n' SHA256SUMS.asc SHA512SUMS.asc
    fi
}

release_signed_assets() {
    release_signed_without_keys_assets
}

require_complete_pgp_config() {
    if [[ -z "${WAITPRIMS_PGP_KEY_ID:-}" || -z "${WAITPRIMS_GPG_HOMEDIR:-}" ]]; then
        echo "error: optional PGP signing requires both PGP variables" >&2
        return 1
    fi
}

assert_exact_directory_inventory() (
    local directory="$1"
    local producer="$2"
    if [[ ! -d "$directory" || -L "$directory" ]]; then
        echo "error: release directory is absent or unsafe" >&2
        return 1
    fi

    local expected_file actual_file
    expected_file="$(mktemp "${TMPDIR:-/tmp}/waitprims-expected.XXXXXX")"
    actual_file="$(mktemp "${TMPDIR:-/tmp}/waitprims-actual.XXXXXX")"
    trap 'rm -f "$expected_file" "$actual_file"' EXIT

    "$producer" | LC_ALL=C sort >"$expected_file"
    find "$directory" -mindepth 1 -maxdepth 1 -print |
        while IFS= read -r entry; do basename "$entry"; done |
        LC_ALL=C sort >"$actual_file"

    if ! cmp -s "$expected_file" "$actual_file"; then
        echo "error: release directory inventory mismatch" >&2
        diff -u "$expected_file" "$actual_file" >&2 || true
        return 1
    fi
    while IFS= read -r name; do
        if [[ ! -f "$directory/$name" || ! -s "$directory/$name" || -L "$directory/$name" ]]; then
            echo "error: release asset is not a regular file: $name" >&2
            return 1
        fi
    done <"$expected_file"
)

assert_github_release_state() (
    local expected_assets_producer="$1" inventory_mode="${2:-exact}"
    local tag repo_path tag_commit
    tag="$(release_tag)"
    repo_path="$(release_repo_root)"
    tag_commit="$(git -C "$repo_path" rev-parse "refs/tags/${tag}^{}")"

    if [[ -n "${GH_REPO+x}" ]]; then
        echo "error: GH_REPO must be unset; repository authority is fixed" >&2
        return 1
    fi
    for command_name in gh jq; do
        command -v "$command_name" >/dev/null 2>&1 || {
            echo "error: required release command is unavailable" >&2
            return 1
        }
    done
    if [[ "$(gh repo view "$WAITPRIMS_REPOSITORY" \
        --json nameWithOwner --jq .nameWithOwner)" != "$WAITPRIMS_REPOSITORY" ]]; then
        echo "error: GitHub repository identity mismatch" >&2
        return 1
    fi

    local state expected actual
    state="$(mktemp "${TMPDIR:-/tmp}/waitprims-release-state.XXXXXX")"
    expected="$(mktemp "${TMPDIR:-/tmp}/waitprims-remote-expected.XXXXXX")"
    actual="$(mktemp "${TMPDIR:-/tmp}/waitprims-remote-actual.XXXXXX")"
    trap 'rm -f "$state" "$expected" "$actual"' EXIT
    gh release view "$tag" --repo "$WAITPRIMS_REPOSITORY" \
        --json tagName,targetCommitish,isDraft,assets >"$state"

    if [[ "$(jq -r .tagName "$state")" != "$tag" ||
    "$(jq -r .targetCommitish "$state")" != "$tag_commit" ||
    "$(jq -r .isDraft "$state")" != "true" ]]; then
        echo "error: GitHub release tag, target, or draft state mismatch" >&2
        return 1
    fi
    "$expected_assets_producer" | LC_ALL=C sort >"$expected"
    jq -r '.assets[].name' "$state" | LC_ALL=C sort >"$actual"
    python3 - "$expected" "$actual" "$inventory_mode" <<'PYTHON'
from pathlib import Path
import sys
expected = Path(sys.argv[1]).read_text().splitlines()
actual = Path(sys.argv[2]).read_text().splitlines()
mode = sys.argv[3]
if len(actual) != len(set(actual)) or mode not in ('exact', 'resume'):
    sys.exit('error: invalid remote inventory or validation mode')
if (mode == 'exact' and actual != expected) or (mode == 'resume' and not set(actual) <= set(expected)):
    sys.exit('error: remote draft asset inventory mismatch')
PYTHON
)

# Bind operator work to the staged annotated object and peeled commit.
require_published_anchor() {
    local directory="$1" anchor tag object commit
    tag="$(release_tag)"
    anchor="$directory.anchor"
    [[ -s "$anchor" && -f "$anchor" && ! -L "$anchor" ]] || {
        echo 'error: staged anchor missing' >&2
        return 1
    }
    [[ "$(awk -F= '$1=="tag" {print $2}' "$anchor")" == "$tag" ]] || {
        echo 'error: staged tag mismatch' >&2
        return 1
    }
    object="$(awk -F= '$1=="object" {print $2}' "$anchor")"
    commit="$(awk -F= '$1=="commit" {print $2}' "$anchor")"
    [[ "$object" =~ ^[0-9a-f]{40}$ && "$commit" =~ ^[0-9a-f]{40}$ ]] || {
        echo 'error: malformed staged identity anchor' >&2
        return 1
    }
    WAITPRIMS_RELEASE_TAG="$tag" WAITPRIMS_EXPECTED_TAG_OBJECT="$object" WAITPRIMS_EXPECTED_COMMIT="$commit" \
        "$(release_repo_root)/scripts/release-verify-published-tag.sh"
}

# The original approved ceremony identity is required across separate registry calls.
load_ceremony_anchor() {
    local anchor="${WAITPRIMS_RELEASE_ANCHOR_FILE:-}" identity canonical repo_path
    [[ "$anchor" == /* && -f "$anchor" && -s "$anchor" && ! -L "$anchor" ]] || {
        echo 'error: approved external ceremony anchor required' >&2
        return 1
    }
    canonical="$(cd "$(dirname "$anchor")" && pwd -P)/$(basename "$anchor")"
    repo_path="$(release_repo_root)"
    case "$canonical" in "$repo_path" | "$repo_path"/*)
        echo 'error: ceremony anchor must be outside trusted checkout' >&2
        return 1
        ;;
    esac
    identity="$(
        python3 - "$anchor" "$(release_tag)" <<'PYTHON'
from pathlib import Path
import re
import sys
entries = {}
for line in Path(sys.argv[1]).read_text().splitlines():
    key, sep, value = line.partition('=')
    if not sep or key in entries or key not in ('tag', 'object', 'commit', 'run', 'attempt'):
        sys.exit('error: malformed ceremony anchor')
    entries[key] = value
if entries.get('tag') != sys.argv[2] or any(not re.fullmatch('[0-9a-f]{40}', entries.get(key, '')) for key in ('object', 'commit')):
    sys.exit('error: ceremony tag/object/commit mismatch or missing')
print(entries['object'], entries['commit'])
PYTHON
    )" || return 1
    read -r WAITPRIMS_EXPECTED_TAG_OBJECT WAITPRIMS_EXPECTED_COMMIT <<<"$identity"
    export WAITPRIMS_EXPECTED_TAG_OBJECT WAITPRIMS_EXPECTED_COMMIT
}

verify_remote_release_bytes() (
    local directory="$1" remote_dir name
    remote_dir="$(mktemp -d)"
    trap 'rm -rf "$remote_dir"' EXIT
    assert_github_release_state release_signed_assets
    gh release download "$(release_tag)" --repo "$WAITPRIMS_REPOSITORY" --pattern '*' --dir "$remote_dir"
    assert_exact_directory_inventory "$remote_dir" release_signed_assets
    while IFS= read -r name; do
        cmp -s "$directory/$name" "$remote_dir/$name" || {
            echo 'error: remote draft asset bytes differ from verified local set' >&2
            return 1
        }
    done < <(release_signed_assets)
)
