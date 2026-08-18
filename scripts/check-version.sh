#!/usr/bin/env bash
# Version consistency check for waitprims
# Validates that VERSION matches Cargo.toml workspace version.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

VERSION_FILE="$PROJECT_ROOT/VERSION"
CARGO_TOML="$PROJECT_ROOT/Cargo.toml"
CHANGELOG="$PROJECT_ROOT/CHANGELOG.md"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

error() {
	echo -e "${RED}[ERROR]${NC} $*" >&2
}

warn() {
	echo -e "${YELLOW}[WARN]${NC} $*" >&2
}

ok() {
	echo -e "${GREEN}[OK]${NC} $*"
}

info() {
	echo "[INFO] $*"
}

if [[ ! -f "$VERSION_FILE" ]]; then
	error "VERSION file not found: $VERSION_FILE"
	exit 1
fi

VERSION_FROM_FILE=$(tr -d '[:space:]' <"$VERSION_FILE")

if [[ -z "$VERSION_FROM_FILE" ]]; then
	error "VERSION file is empty"
	exit 1
fi

if ! echo "$VERSION_FROM_FILE" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.-]+)?(\+[a-zA-Z0-9.-]+)?$'; then
	error "VERSION file contains invalid semver: $VERSION_FROM_FILE"
	exit 1
fi

ok "VERSION file: $VERSION_FROM_FILE"

if [[ ! -f "$CARGO_TOML" ]]; then
	error "Cargo.toml not found: $CARGO_TOML"
	exit 1
fi

VERSION_FROM_CARGO=$(grep -A 20 '^\[workspace\.package\]' "$CARGO_TOML" | grep '^version' | head -1 | sed 's/.*"\(.*\)".*/\1/')

if [[ -z "$VERSION_FROM_CARGO" ]]; then
	error "Could not extract version from Cargo.toml [workspace.package]"
	exit 1
fi

ok "Cargo.toml workspace version: $VERSION_FROM_CARGO"

if [[ "$VERSION_FROM_FILE" != "$VERSION_FROM_CARGO" ]]; then
	error "Version mismatch!"
	error "  VERSION file:    $VERSION_FROM_FILE"
	error "  Cargo.toml:      $VERSION_FROM_CARGO"
	error ""
	error "Run 'make version-sync' to sync Cargo.toml to VERSION file"
	exit 1
fi

ok "VERSION matches Cargo.toml workspace version"

if [[ ! -f "$CHANGELOG" ]]; then
	warn "CHANGELOG.md not found: $CHANGELOG"
else
	if grep -qE "^## \[?${VERSION_FROM_FILE}\]?" "$CHANGELOG"; then
		ok "CHANGELOG.md has entry for $VERSION_FROM_FILE"
	else
		error "CHANGELOG.md does not have an entry for $VERSION_FROM_FILE"
		exit 1
	fi
fi

info "Checking crate Cargo.toml files..."

FAILED_CRATES=()

for crate_dir in "$PROJECT_ROOT"/crates/*; do
	if [[ ! -d "$crate_dir" ]]; then
		continue
	fi

	crate_name=$(basename "$crate_dir")
	crate_toml="$crate_dir/Cargo.toml"

	if [[ ! -f "$crate_toml" ]]; then
		warn "Cargo.toml not found for crate: $crate_name"
		continue
	fi

	if grep -A 10 '^\[package\]' "$crate_toml" | grep -q '^version\.workspace\s*=\s*true'; then
		ok "  $crate_name: using workspace version"
	else
		if grep -A 10 '^\[package\]' "$crate_toml" | grep -q '^version\s*='; then
			error "  $crate_name: uses hardcoded version instead of workspace"
			FAILED_CRATES+=("$crate_name")
		else
			warn "  $crate_name: no version field found"
		fi
	fi
done

if [[ ${#FAILED_CRATES[@]} -gt 0 ]]; then
	error ""
	error "The following crates have hardcoded versions instead of version.workspace = true:"
	for crate in "${FAILED_CRATES[@]}"; do
		error "  - $crate"
	done
	error ""
	error "Update their Cargo.toml to use: version.workspace = true"
	exit 1
fi

ok "All crates use workspace version"

DEP_MISMATCH=0
while IFS= read -r line; do
	dep_ver=$(echo "$line" | sed 's/.*version = "\([^"]*\)".*/\1/')
	if [[ "$dep_ver" != "$VERSION_FROM_FILE" ]]; then
		error "workspace dependency version $dep_ver does not match VERSION $VERSION_FROM_FILE"
		error "  $line"
		DEP_MISMATCH=1
	fi
done < <(grep -E '^waitprims-(core|async|testkit) = \{ version = "' "$CARGO_TOML" || true)

if [[ "$DEP_MISMATCH" -ne 0 ]]; then
	error "Run 'make version-sync' to sync path-dependency versions"
	exit 1
fi

ok "Workspace path-dependency versions match"

echo ""
ok "Version consistency check passed"
