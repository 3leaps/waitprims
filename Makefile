# waitprims Makefile
# Rust-only wait primitive. The CLI is diagnostic.
#
# Quick reference:
#   make help              Show available targets
#   make precommit         fmt-check, clippy
#   make prepush           fmt-check, clippy, locked tests, version-check
#   make pr-final          same as prepush (PR merge-readiness)
#   make version-sync      Sync VERSION to Cargo.toml
#   make release-preflight Verify pre-tag requirements
#   make release-check     Version consistency + cargo package (does not publish)

.PHONY: all help check test fmt fmt-check lint build clean version demo-follow demo-coalesce demo-watch
.PHONY: precommit prepush pr-final
.PHONY: version-patch version-minor version-major version-set version-sync version-check
.PHONY: release-check release-preflight release-guard-tag-version
.PHONY: release-clean release-download release-checksums release-sign release-export-keys
.PHONY: release-verify release-verify-checksums release-verify-signatures release-verify-keys
.PHONY: release-notes release-upload release

# Release stages must not overlap under `make -j`.
.NOTPARALLEL:

# -----------------------------------------------------------------------------
# Configuration
# -----------------------------------------------------------------------------

VERSION_FILE := VERSION
VERSION := $(shell tr -d ' \t\r\n' < $(VERSION_FILE) 2>/dev/null || echo dev)

CARGO = cargo

DIST_RELEASE := dist/release
# In-tree VERSION is the default tag, not the nearest older git tag.
WAITPRIMS_RELEASE_TAG ?= v$(VERSION)
export WAITPRIMS_RELEASE_TAG

WAITPRIMS_MINISIGN_KEY ?=
WAITPRIMS_MINISIGN_PUB ?=
WAITPRIMS_PGP_KEY_ID ?=
WAITPRIMS_GPG_HOMEDIR ?=

# -----------------------------------------------------------------------------
# Default and Help
# -----------------------------------------------------------------------------

all: check

help: ## Show available targets
	@echo "waitprims - library-first wait primitive"
	@echo "The CLI is diagnostic. There is no daemon and no bindings."
	@echo ""
	@echo "Quality gates:"
	@echo "  help            Show this help message"
	@echo "  check           fmt-check, clippy, locked tests"
	@echo "  test            cargo test --workspace --locked"
	@echo "  fmt             cargo fmt --all"
	@echo "  fmt-check       cargo fmt --all -- --check"
	@echo "  lint            cargo clippy --workspace --all-targets -- -D warnings"
	@echo "  build           cargo build --workspace"
	@echo "  demo-follow     Locked offline held-follow CLI demo vs golden JSONL"
	@echo "  demo-coalesce   Locked offline held-coalesce CLI demo vs golden JSONL"
	@echo "  demo-watch      Native filesystem CLI demo with bounded retry"
	@echo "  clean           cargo clean"
	@echo "  precommit       fmt-check, clippy"
	@echo "  prepush         fmt-check, clippy, locked tests, version-check"
	@echo "  pr-final        same as prepush (PR merge-readiness)"
	@echo ""
	@echo "Release:"
	@echo "  release-preflight  Verify all pre-tag requirements (REQUIRED before tagging)"
	@echo "  release-check      Version consistency + cargo package (does not publish)"
	@echo "  release-clean      Remove dist/release contents"
	@echo "  release-download   Download release assets from GitHub"
	@echo "  release-checksums  Generate SHA256SUMS and SHA512SUMS"
	@echo "  release-sign       Sign checksum manifests (minisign + PGP)"
	@echo "  release-export-keys Export public signing keys"
	@echo "  release-verify     Verify checksums, signatures, and keys"
	@echo "  release-notes      Copy docs/releases/vX.Y.Z.md into dist (before checksums)"
	@echo "  release-upload     Upload signed artifacts to GitHub"
	@echo "  release            Full signing workflow (clean -> upload)"
	@echo ""
	@echo "Version management:"
	@echo "  version         Print current version"
	@echo "  version-check   Validate version consistency across files"
	@echo "  version-patch   Bump patch version (0.1.0 -> 0.1.1)"
	@echo "  version-minor   Bump minor version (0.1.0 -> 0.2.0)"
	@echo "  version-major   Bump major version (0.1.0 -> 1.0.0)"
	@echo "  version-set     Set explicit version (V=X.Y.Z)"
	@echo "  version-sync    Sync VERSION to Cargo.toml"
	@echo ""
	@echo "Current version: $(VERSION)"

# -----------------------------------------------------------------------------
# Quality Gates
# -----------------------------------------------------------------------------

check: fmt-check lint test ## Run quality checks
	@echo "[ok] All quality checks passed"

test: ## Run locked test suite
	@echo "Running tests..."
	$(CARGO) test --workspace --locked
	@echo "[ok] Tests passed"

fmt: ## Format Rust
	$(CARGO) fmt --all
	@echo "[ok] Formatting complete"

fmt-check: ## Check formatting without modifying
	$(CARGO) fmt --all -- --check
	@echo "[ok] Formatting check passed"

lint: ## Run clippy
	$(CARGO) clippy --workspace --all-targets -- -D warnings
	@echo "[ok] Linting passed"

build: ## Build all crates (debug)
	$(CARGO) build --workspace
	@echo "[ok] Build complete"

demo-follow: ## Locked offline held-follow CLI demo vs golden JSONL
	$(CARGO) build --locked --offline -p waitprims-cli
	./target/debug/waitprims --log-level error follow \
		--registration-set fixtures/follow-demo/registration_set.json \
		--request fixtures/follow-demo/live_wait_request.json \
		--script fixtures/follow-demo/follow.json \
		| cmp - fixtures/follow-demo/golden.jsonl
	@echo "[ok] demo-follow matched golden JSONL"

demo-coalesce: ## Locked offline held-coalesce CLI demo vs golden JSONL
	$(CARGO) build --locked --offline -p waitprims-cli
	./target/debug/waitprims --log-level error coalesce \
		--registration-set fixtures/coalesce-demo/registration_set.json \
		--request fixtures/coalesce-demo/live_wait_request.json \
		--script fixtures/coalesce-demo/coalesce.json \
		| cmp - fixtures/coalesce-demo/golden.jsonl
	@echo "[ok] demo-coalesce matched golden JSONL"

demo-watch: ## Native filesystem CLI demo with bounded create/remove retry
	$(CARGO) test --locked -p waitprims-cli --test watch_cli native_watch_demo_uses_bounded_retry_and_visible_event_surface
	@echo "[ok] demo-watch observed native filesystem JSONL"

clean: ## Remove build artifacts
	$(CARGO) clean
	@echo "[ok] Clean complete"

precommit: fmt-check lint ## Fast checks for every commit
	@echo "[ok] Pre-commit checks passed"

prepush: fmt-check lint test version-check ## Thorough checks before push
	@echo "[ok] Pre-push checks passed"

pr-final: prepush demo-follow demo-coalesce demo-watch ## Final PR merge-readiness gate
	@echo "[ok] PR final checks passed"

# -----------------------------------------------------------------------------
# Version Management
# -----------------------------------------------------------------------------
#
# Version SSOT is the VERSION file (not Cargo.toml).
# Cargo.toml workspace version should match VERSION.

version: ## Print current version
	@echo "$(VERSION)"

version-patch: ## Bump patch version (0.1.0 -> 0.1.1)
	@current=$$(cat $(VERSION_FILE)); \
	major=$$(echo $$current | cut -d. -f1); \
	minor=$$(echo $$current | cut -d. -f2); \
	patch=$$(echo $$current | cut -d. -f3); \
	new_patch=$$((patch + 1)); \
	new_version="$$major.$$minor.$$new_patch"; \
	echo "$$new_version" > $(VERSION_FILE); \
	echo "Version bumped: $$current -> $$new_version"

version-minor: ## Bump minor version (0.1.0 -> 0.2.0)
	@current=$$(cat $(VERSION_FILE)); \
	major=$$(echo $$current | cut -d. -f1); \
	minor=$$(echo $$current | cut -d. -f2); \
	new_minor=$$((minor + 1)); \
	new_version="$$major.$$new_minor.0"; \
	echo "$$new_version" > $(VERSION_FILE); \
	echo "Version bumped: $$current -> $$new_version"

version-major: ## Bump major version (0.1.0 -> 1.0.0)
	@current=$$(cat $(VERSION_FILE)); \
	major=$$(echo $$current | cut -d. -f1); \
	new_major=$$((major + 1)); \
	new_version="$$new_major.0.0"; \
	echo "$$new_version" > $(VERSION_FILE); \
	echo "Version bumped: $$current -> $$new_version"

version-set: ## Set explicit version (V=X.Y.Z)
	@if [ -z "$(V)" ]; then \
		echo "Usage: make version-set V=1.2.3"; \
		exit 1; \
	fi
	@echo "$(V)" > $(VERSION_FILE)
	@echo "Version set to $(V)"

version-sync: ## Sync VERSION file to Cargo.toml
	@ver=$$(tr -d ' \t\r\n' < $(VERSION_FILE)); \
	if cargo set-version -V >/dev/null 2>&1; then \
		cargo set-version --workspace "$$ver"; \
		echo "[ok] Synced Cargo.toml to $$ver"; \
	else \
		python3 -c "\
import pathlib, re, sys; \
ver = sys.argv[1]; \
p = pathlib.Path('Cargo.toml'); \
text = p.read_text(); \
text, n = re.subn(r'(?m)^version = \"[^\"]*\"', 'version = \"%s\"' % ver, text, count=1); \
if n != 1: \
    raise SystemExit('failed to update [workspace.package] version'); \
text = re.sub(r'(waitprims-(?:core|async|testkit) = \{ version = )\"[^\"]*\"', r'\1\"%s\"' % ver, text); \
p.write_text(text); \
" "$$ver"; \
		echo "[ok] Synced Cargo.toml to $$ver (python fallback)"; \
	fi

version-check: ## Validate version consistency across files
	@echo "Checking version consistency..."
	@./scripts/check-version.sh

# -----------------------------------------------------------------------------
# Release
# -----------------------------------------------------------------------------
#
# Workflow:
# 1. Pre-tag: make release-preflight
# 2. Tag and push: git tag vX.Y.Z && git push origin vX.Y.Z
# 3. Wait for GitHub Actions release workflow to create a draft release
# 4. Sign locally: make release (or individual leaf targets)
#
# Leaf targets have no write-chain precursors (same as sysprims).
# Only `make release` walks clean → download → notes → checksums →
# sign → export-keys → upload (verify once via upload).
# `make release-export-keys` must not re-clean or re-download.
#
# Environment variables:
#   WAITPRIMS_MINISIGN_KEY  - Path to minisign secret key (required for sign)
#   WAITPRIMS_MINISIGN_PUB  - Path to minisign public key (optional)
#   WAITPRIMS_PGP_KEY_ID    - PGP key ID for GPG signing (optional)
#   WAITPRIMS_GPG_HOMEDIR   - Custom GPG home directory (optional)
#
# CI never holds signing keys. MFA / hardware-token signing is local.

release-check: version-check ## Version consistency + package check (does not publish)
	@echo "Checking release readiness..."
	@echo ""
	@echo "Packaging workspace crates (does not cargo publish)..."
	@# Same gate as ipcprims: cargo package --workspace verifies
	@# dependents from a local tmp registry. Workspace publish stays
	@# false; libraries opt in. The CLI is packaged here but not
	@# publishable.
	@$(CARGO) package --workspace
	@echo "[ok] Package check passed"
	@echo ""
	@echo "Release checklist:"
	@echo "  ✓ Version consistency validated"
	@echo "  ✓ Package check passed"
	@echo "  ✓ cargo publish was not run"
	@echo ""
	@echo "Next steps:"
	@echo "  1. make release-preflight"
	@echo "  2. git tag v$$(tr -d ' \t\r\n' < $(VERSION_FILE))"
	@echo "  3. git push origin v$$(tr -d ' \t\r\n' < $(VERSION_FILE))"
	@echo "  4. Wait for CI + release workflow"
	@echo "  5. make release (sign + upload)"

release-preflight: ## Verify all pre-tag requirements (REQUIRED before tagging)
	@echo "Running release preflight checks..."
	@echo ""
	@if [ -n "$$(git status --porcelain 2>/dev/null)" ]; then \
		echo "[!!] Working tree not clean - commit or stash changes first"; \
		git status --short; \
		exit 1; \
	fi
	@echo "[ok] Working tree is clean"
	@$(MAKE) prepush --silent
	@echo "[ok] Prepush checks passed"
	@$(MAKE) version-check --silent
	@echo "[ok] Version synced"
	@version_file=$$(tr -d ' \t\r\n' < $(VERSION_FILE)); \
	if [ ! -f RELEASE_NOTES.md ]; then \
		echo "[!!] RELEASE_NOTES.md not found"; \
		exit 1; \
	fi; \
	if ! grep -qE "^## v$$version_file( |$$)" RELEASE_NOTES.md; then \
		echo "[!!] RELEASE_NOTES.md has no heading for v$$version_file"; \
		exit 1; \
	fi; \
	echo "[ok] RELEASE_NOTES.md has v$$version_file"; \
	cut_notes="docs/releases/v$$version_file.md"; \
	if [ ! -f "$$cut_notes" ]; then \
		echo "[!!] Per-cut notes not found at $$cut_notes"; \
		echo "    Extract the v$$version_file section from RELEASE_NOTES.md"; \
		exit 1; \
	fi; \
	if ! grep -qE "^# v$$version_file( |$$)" "$$cut_notes"; then \
		echo "[!!] $$cut_notes must start from the v$$version_file cut"; \
		exit 1; \
	fi; \
	other=$$(grep -E "^#{1,2} v[0-9]+\.[0-9]+\.[0-9]+" "$$cut_notes" | grep -vE "v$$version_file( |$$)" || true); \
	if [ -n "$$other" ]; then \
		echo "[!!] $$cut_notes must contain only the v$$version_file section:"; \
		echo "$$other"; \
		exit 1; \
	fi
	@echo "[ok] Per-cut notes exist"
	@echo "[..] Verifying local/remote sync..."
	@if ! git fetch origin; then \
		echo "[!!] git fetch origin failed; cannot verify local/remote sync"; \
		exit 1; \
	fi
	@local_only=$$(git log --oneline origin/main..HEAD 2>/dev/null | wc -l | tr -d ' '); \
	remote_only=$$(git log --oneline HEAD..origin/main 2>/dev/null | wc -l | tr -d ' '); \
	if [ "$$local_only" -gt 0 ] || [ "$$remote_only" -gt 0 ]; then \
		echo "[!!] Local and remote are out of sync"; \
		if [ "$$local_only" -gt 0 ]; then \
			echo "    $$local_only local commit(s) not pushed"; \
		fi; \
		if [ "$$remote_only" -gt 0 ]; then \
			echo "    $$remote_only remote commit(s) not pulled"; \
		fi; \
		exit 1; \
	fi
	@echo "[ok] Local and remote are in sync"
	@echo ""
	@echo "[ok] All preflight checks passed - ready to tag"
	@version_file=$$(tr -d ' \t\r\n' < $(VERSION_FILE)); \
	echo "    Next: git tag \"v$$version_file\" -m \"Release $$version_file\""

release-guard-tag-version: ## Verify tag matches VERSION file
	./scripts/release-guard-tag-version.sh

release-clean: ## Remove dist/release contents
	@echo "Cleaning release directory..."
	rm -rf $(DIST_RELEASE)
	@echo "[ok] Release directory cleaned"

release-download: ## Download release assets from GitHub
	@if [ -z "$(WAITPRIMS_RELEASE_TAG)" ] || [ "$(WAITPRIMS_RELEASE_TAG)" = "v" ]; then \
		echo "Error: No release tag found. Set WAITPRIMS_RELEASE_TAG=vX.Y.Z"; \
		exit 1; \
	fi
	./scripts/download-release-assets.sh $(WAITPRIMS_RELEASE_TAG) $(DIST_RELEASE)

release-notes: ## Copy docs/releases/vX.Y.Z.md into dist before checksums
	@src="docs/releases/$(WAITPRIMS_RELEASE_TAG).md"; \
	if [ ! -f "$$src" ]; then \
		echo "[!!] Per-cut notes not found at $$src"; \
		echo "    Extract that version's section from RELEASE_NOTES.md"; \
		exit 1; \
	fi; \
	mkdir -p "$(DIST_RELEASE)"; \
	cp "$$src" "$(DIST_RELEASE)/release-notes-$(WAITPRIMS_RELEASE_TAG).md"; \
	echo "[ok] Copied $$src into the checksum set"

release-checksums: ## Generate SHA256SUMS and SHA512SUMS
	./scripts/generate-checksums.sh $(DIST_RELEASE) $(WAITPRIMS_RELEASE_TAG)

release-sign: ## Sign checksum manifests (requires WAITPRIMS_MINISIGN_KEY)
	@if [ -z "$(WAITPRIMS_MINISIGN_KEY)" ]; then \
		echo "Error: WAITPRIMS_MINISIGN_KEY not set"; \
		echo ""; \
		echo "Set the path to your minisign secret key:"; \
		echo "  export WAITPRIMS_MINISIGN_KEY=/path/to/signing.key"; \
		exit 1; \
	fi
	WAITPRIMS_MINISIGN_KEY=$(WAITPRIMS_MINISIGN_KEY) \
	WAITPRIMS_PGP_KEY_ID=$(WAITPRIMS_PGP_KEY_ID) \
	WAITPRIMS_GPG_HOMEDIR=$(WAITPRIMS_GPG_HOMEDIR) \
	./scripts/sign-release-assets.sh $(WAITPRIMS_RELEASE_TAG) $(DIST_RELEASE)

release-export-keys: ## Export public signing keys
	WAITPRIMS_MINISIGN_KEY=$(WAITPRIMS_MINISIGN_KEY) \
	WAITPRIMS_MINISIGN_PUB=$(WAITPRIMS_MINISIGN_PUB) \
	WAITPRIMS_PGP_KEY_ID=$(WAITPRIMS_PGP_KEY_ID) \
	WAITPRIMS_GPG_HOMEDIR=$(WAITPRIMS_GPG_HOMEDIR) \
	./scripts/export-release-keys.sh $(DIST_RELEASE)

release-verify-checksums: ## Verify checksums match artifacts
	@echo "Verifying checksums..."
	cd $(DIST_RELEASE) && shasum -a 256 -c SHA256SUMS
	@echo "[ok] Checksums verified"

release-verify-signatures: ## Verify minisign/PGP signatures
	./scripts/verify-signatures.sh $(DIST_RELEASE)

release-verify-keys: ## Verify exported keys are public-only
	./scripts/verify-public-keys.sh $(DIST_RELEASE)

release-verify: release-verify-checksums release-verify-signatures release-verify-keys ## Run all release verification
	@echo "[ok] All release verifications passed"

release-upload: release-verify ## Upload signed artifacts to GitHub release
	./scripts/upload-release-assets.sh $(WAITPRIMS_RELEASE_TAG) $(DIST_RELEASE)

# Serialized walk only. Leaves stay independent so mid-chain targets
# do not re-run clean/download. `.NOTPARALLEL` still blocks `make -j`.
# Verify runs once, as the `release-upload` grouping prerequisite.
release: release-guard-tag-version ## Full signing workflow (after CI build)
	$(MAKE) release-clean
	$(MAKE) release-download
	$(MAKE) release-notes
	$(MAKE) release-checksums
	$(MAKE) release-sign
	$(MAKE) release-export-keys
	$(MAKE) release-upload
	@echo "[ok] Release $(WAITPRIMS_RELEASE_TAG) complete"
