#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GUARD="$SCRIPT_DIR/release-guard-tag-version.sh"
TEST_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/waitprims-release-guard.XXXXXX")"
trap 'rm -rf "$TEST_ROOT"' EXIT

cp "$GUARD" "$TEST_ROOT/release-guard-tag-version.sh"
chmod +x "$TEST_ROOT/release-guard-tag-version.sh"
printf '1.2.3\n' >"$TEST_ROOT/VERSION"

git -C "$TEST_ROOT" init -q
git -C "$TEST_ROOT" config user.name "waitprims release guard test"
git -C "$TEST_ROOT" config user.email "noreply@example.invalid"
git -C "$TEST_ROOT" add VERSION release-guard-tag-version.sh
git -C "$TEST_ROOT" commit -qm "test fixture"
git -C "$TEST_ROOT" tag -a v1.2.3 -m "test fixture"

run_guard() {
	(
		cd "$TEST_ROOT"
		env -u WAITPRIMS_RELEASE_KEY \
			-u WAITPRIMS_RELEASE_TAG \
			-u WAITPRIMS_REQUIRE_TAG \
			-u RELEASE_TAG \
			"$@"
	)
}

expect_fail() {
	if "$@" >/dev/null 2>&1; then
		echo "expected command to fail: $*" >&2
		exit 1
	fi
}

run_guard "$TEST_ROOT/release-guard-tag-version.sh" >/dev/null
run_guard WAITPRIMS_RELEASE_KEY=v1.2.3 \
	"$TEST_ROOT/release-guard-tag-version.sh" >/dev/null
run_guard WAITPRIMS_RELEASE_KEY=v1.2.3 WAITPRIMS_REQUIRE_TAG=1 \
	"$TEST_ROOT/release-guard-tag-version.sh" >/dev/null
run_guard VERSION=v9.9.9 WAITPRIMS_RELEASE_KEY=v1.2.3 \
	"$TEST_ROOT/release-guard-tag-version.sh" >/dev/null

expect_fail run_guard WAITPRIMS_RELEASE_KEY=1.2.3 \
	"$TEST_ROOT/release-guard-tag-version.sh"
expect_fail run_guard WAITPRIMS_RELEASE_KEY=vv1.2.3 \
	"$TEST_ROOT/release-guard-tag-version.sh"
expect_fail run_guard WAITPRIMS_RELEASE_KEY=v1.2.3 \
	WAITPRIMS_RELEASE_TAG=vv1.2.3 \
	"$TEST_ROOT/release-guard-tag-version.sh"

printf 'untagged\n' >"$TEST_ROOT/marker"
git -C "$TEST_ROOT" add marker
git -C "$TEST_ROOT" commit -qm "untagged fixture"
run_guard "$TEST_ROOT/release-guard-tag-version.sh" >/dev/null
expect_fail run_guard WAITPRIMS_RELEASE_KEY=v1.2.3 \
	WAITPRIMS_REQUIRE_TAG=1 \
	"$TEST_ROOT/release-guard-tag-version.sh"

printf '1.2.4\n' >"$TEST_ROOT/VERSION"
git -C "$TEST_ROOT" add VERSION
git -C "$TEST_ROOT" commit -qm "lightweight tag fixture"
git -C "$TEST_ROOT" tag v1.2.4
expect_fail run_guard WAITPRIMS_RELEASE_KEY=v1.2.4 \
	WAITPRIMS_REQUIRE_TAG=1 \
	"$TEST_ROOT/release-guard-tag-version.sh"

echo "[ok] release tag/version guard tests passed"
