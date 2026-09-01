#!/usr/bin/env bash
# release-guard-tag-version.sh - Verify tag matches VERSION file
#
# Use in CI to ensure version consistency, or before signing locally.
#
# Environment variables:
#   WAITPRIMS_RELEASE_KEY   - Operator-provided release key (v-prefixed tag)
#   WAITPRIMS_RELEASE_TAG   - Override tag to check
#   WAITPRIMS_REQUIRE_TAG   - Set to 1 to fail if no tag found (for CI)

set -euo pipefail

repo_root() {
	git rev-parse --show-toplevel
}

read_version() {
	if [ ! -f VERSION ]; then
		echo "error: VERSION file not found" >&2
		exit 1
	fi
	tr -d ' \t\r\n' <VERSION
}

detect_tag() {
	if [ -n "${WAITPRIMS_RELEASE_TAG:-}" ]; then
		printf '%s' "${WAITPRIMS_RELEASE_TAG}"
		return 0
	fi
	if [ -n "${WAITPRIMS_RELEASE_KEY:-}" ]; then
		printf '%s' "${WAITPRIMS_RELEASE_KEY}"
		return 0
	fi
	if [ -n "${RELEASE_TAG:-}" ]; then
		printf '%s' "${RELEASE_TAG}"
		return 0
	fi
	# Exact tag on HEAD if present. Never the nearest older tag.
	local exact
	exact="$(git describe --tags --exact-match 2>/dev/null || true)"
	if [ -n "$exact" ]; then
		printf '%s' "$exact"
		return 0
	fi
	# Untagged HEAD: target the in-tree VERSION, not an older tag.
	printf 'v%s' "$(read_version)"
}

main() {
	local root
	root="$(repo_root)"
	cd "$root"

	local version
	version="$(read_version)"

	local expected="v${version}"
	if [ -n "${WAITPRIMS_RELEASE_TAG:-}" ] &&
		[ -n "${WAITPRIMS_RELEASE_KEY:-}" ] &&
		[ "${WAITPRIMS_RELEASE_TAG}" != "${WAITPRIMS_RELEASE_KEY}" ]; then
		echo "error: conflicting release tag inputs" >&2
		echo "  WAITPRIMS_RELEASE_TAG and WAITPRIMS_RELEASE_KEY differ" >&2
		exit 1
	fi

	local tag
	tag="$(detect_tag)"

	if [ -z "$tag" ]; then
		local require_tag="${WAITPRIMS_REQUIRE_TAG:-}"
		if [ "${require_tag}" = "1" ]; then
			echo "error: no exact tag found and no explicit release key/tag provided" >&2
			exit 1
		fi
		echo "[--] release guard: no tag detected (set WAITPRIMS_REQUIRE_TAG=1 to enforce in CI)"
		exit 0
	fi

	if [ "$tag" != "$expected" ]; then
		echo "error: release tag/version mismatch" >&2
		echo "  tag:     $tag" >&2
		echo "  VERSION: $version (expected tag: $expected)" >&2
		exit 1
	fi

	if [ "${WAITPRIMS_REQUIRE_TAG:-}" = "1" ]; then
		if ! git tag --points-at HEAD --format='%(refname:short)' |
			grep -Fqx "$tag"; then
			echo "error: required release tag is not on HEAD: $tag" >&2
			exit 1
		fi
		if [ "$(git cat-file -t "refs/tags/$tag" 2>/dev/null || true)" != "tag" ]; then
			echo "error: required release tag is not annotated: $tag" >&2
			exit 1
		fi
	fi

	echo "[ok] release guard: tag matches VERSION ($tag)"
}

main "$@"
