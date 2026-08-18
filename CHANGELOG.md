# Changelog

All notable changes to this project are documented in this file.

## [Unreleased]

## [0.1.2]

Release-kit hygiene. No library API change is intended. This
release does not publish to crates.io or change visibility.

- Makefile release leaf targets no longer depend on earlier write
  stages. `make release-export-keys` does not re-clean or re-download
- `make release` is the only serialized walk: clean, download, notes,
  checksums, sign, export-keys, upload (verify once via upload)
- CLI `--version` test tracks the workspace `VERSION` file so a
  patch bump does not require a hardcoded pin
- Makefile adds `precommit` and `pr-final` beside the existing
  `prepush` gate

## [0.1.1]

Signed-release kit. No library API change is intended. Workspace version
is 0.1.1 so a later maintainer tag can cut a signed GitHub release. This
commit does not create a tag, publish to crates.io, or change visibility.

- VERSION is the version SSOT (`make version-sync`, `make version-check`)
- Makefile release targets and `scripts/` for download, checksums, MFA
  sign, verify, and upload (`WAITPRIMS_*` env vars)
- Draft GitHub release workflow on `v*` tags: CLI archives for the five
  smoked platforms, SBOM, `LICENSE-MIT`, `LICENSE-APACHE`
- CI does not hold signing keys; `make release-check` packages the
  workspace and does not run `cargo publish`
- `publish` stays `false`; repository/homepage added for a later crates.io
  enablement
- Rust only: no Go/TS/FFI bindings half

## [0.1.0]

First tagged library. Pin from git (`tag = "v0.1.0"`). Not on crates.io.
APIs may still move.

- Library-first wait primitive: first-match when the caller can block,
  one bounded poll-cycle when they cannot
- Public JSON is the six `agent-wait/v0` message kinds
- `waitprims-testkit` FakeClock and scripted observers
- Diagnostic CLI (`waitprims`)
- CI matrix already on main (`fast` plus hosted smokes; arm64 labs
  same-repo only)

[Unreleased]: https://github.com/3leaps/waitprims/compare/v0.1.2...HEAD
[0.1.2]: https://github.com/3leaps/waitprims/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/3leaps/waitprims/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/3leaps/waitprims/releases/tag/v0.1.0
