#!/usr/bin/env bash
# Cursor Cloud Agent install script for the prims workspace.
#
# Primary repo: waitprims (this repo). Optional sibling repos (crucible,
# 3leaps-productbook-internal) are set up when present. The script is
# idempotent and never hard-fails on an absent sibling repo, so a single-repo
# checkout still succeeds and a multi-repo checkout is fully provisioned.
set -euo pipefail

log() { printf '\n=== %s ===\n' "$*"; }

# --- Locate repos -----------------------------------------------------------
# This script lives at <waitprims>/.cursor/install.sh; Cursor runs `install`
# from the primary repo root, but resolve paths from the script location so it
# works regardless of the current working directory.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WAITPRIMS_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
REPOS_ROOT="$(cd "$WAITPRIMS_DIR/.." && pwd)"

find_repo() { # $1 = directory name, $2 = marker file that must exist inside it
    local base
    for base in "$REPOS_ROOT" "$HOME" /agent/repos /workspace; do
        if [ -f "$base/$1/$2" ]; then
            (cd "$base/$1" && pwd)
            return 0
        fi
    done
    return 1
}
CRUCIBLE_DIR="$(find_repo crucible package.json || true)"
PRODUCTBOOK_DIR="$(find_repo 3leaps-productbook-internal site.yaml || true)"

export PATH="$HOME/.local/bin:$HOME/.bun/bin:$PATH"
mkdir -p "$HOME/.local/bin"

# --- Helpers ----------------------------------------------------------------
SUDO=""
if command -v sudo >/dev/null 2>&1 && sudo -n true 2>/dev/null; then SUDO="sudo"; fi

apt_install() { # best-effort; system deps may already be baked into the image
    if command -v apt-get >/dev/null 2>&1 && [ -n "$SUDO" ]; then
        $SUDO apt-get update -qq || true
        $SUDO apt-get install -y -qq "$@" || true
    fi
}

# --- System packages --------------------------------------------------------
# minisign is the trust anchor for sfetch (which installs goneat/kitfly).
# The linters yamllint and shellcheck back goneat's lint category.
command -v minisign >/dev/null 2>&1 || apt_install minisign
command -v shellcheck >/dev/null 2>&1 || apt_install shellcheck
command -v yamllint >/dev/null 2>&1 || apt_install yamllint

# --- Rust toolchain ---------------------------------------------------------
# waitprims pins rust-version 1.88.0; the default image ships an older stable.
if command -v rustup >/dev/null 2>&1; then
    log "rust toolchain 1.88.0"
    rustup toolchain list | grep -q '^1\.88\.0' ||
        rustup toolchain install 1.88.0 --profile minimal --component rustfmt --component clippy
    rustup default 1.88.0
fi

# --- bun (crucible) ---------------------------------------------------------
if ! command -v bun >/dev/null 2>&1; then
    log "bun"
    export BUN_INSTALL="$HOME/.bun"
    curl -fsSL https://bun.sh/install | bash
fi
export BUN_INSTALL="${BUN_INSTALL:-$HOME/.bun}"

# --- sfetch trust anchor ----------------------------------------------------
# NOTE: the "latest" GitHub download path is /releases/latest/download/... ,
# not /releases/download/latest/... (the crucible Makefile uses the latter and
# 404s), so install sfetch directly here.
if ! command -v sfetch >/dev/null 2>&1; then
    log "sfetch"
    curl -fsSL https://github.com/3leaps/sfetch/releases/latest/download/install-sfetch.sh -o /tmp/install-sfetch.sh
    bash /tmp/install-sfetch.sh --dir "$HOME/.local/bin" --yes
fi

# --- goneat (fmt + lint across all repos) -----------------------------------
if ! command -v goneat >/dev/null 2>&1; then
    log "goneat"
    (cd "$HOME/.local/bin" && sfetch --repo fulmenhq/goneat --tag v0.5.1)
    chmod +x "$HOME/.local/bin/goneat" 2>/dev/null || true
fi

# --- Foundation Go tools (goneat format/lint helpers) -----------------------
export GOBIN="$HOME/.local/bin"
if command -v go >/dev/null 2>&1; then
    command -v yamlfmt >/dev/null 2>&1 || go install github.com/google/yamlfmt/cmd/yamlfmt@latest || true
    command -v shfmt >/dev/null 2>&1 || go install mvdan.cc/sh/v3/cmd/shfmt@latest || true
    command -v checkmake >/dev/null 2>&1 || go install github.com/checkmake/checkmake/cmd/checkmake@latest || true
    command -v actionlint >/dev/null 2>&1 || go install github.com/rhysd/actionlint/cmd/actionlint@latest || true
fi

# --- kitfly (productbook docs site) -----------------------------------------
if [ -n "$PRODUCTBOOK_DIR" ] && ! command -v kitfly >/dev/null 2>&1; then
    log "kitfly"
    (cd "$HOME/.local/bin" && sfetch --repo 3leaps/kitfly --install) || true
    [ -f "$HOME/.local/bin/kitfly-linux-amd64" ] &&
        ln -sf "$HOME/.local/bin/kitfly-linux-amd64" "$HOME/.local/bin/kitfly"
fi

# --- Primary repo: waitprims (Rust) -----------------------------------------
log "waitprims: fetch + build"
(cd "$WAITPRIMS_DIR" && cargo fetch --locked && cargo build --workspace)

# --- crucible: bun dependencies ---------------------------------------------
if [ -n "$CRUCIBLE_DIR" ]; then
    log "crucible: bun install"
    (cd "$CRUCIBLE_DIR" && bun install)
fi

log "install complete"
echo "primary : $WAITPRIMS_DIR"
echo "crucible: ${CRUCIBLE_DIR:-<not checked out>}"
echo "prodbook: ${PRODUCTBOOK_DIR:-<not checked out>}"
