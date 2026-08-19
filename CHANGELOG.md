# Changelog

All notable changes to this project are documented in this file.

Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
This project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

> **Purge policy:** this file retains the **latest 10 releases**.
> Older entries are archived in `docs/releases/v<semver>.md` and
> removed from this file. The landing page (`RELEASE_NOTES.md`)
> keeps only the latest three cuts.

## [Unreleased]

- `run_follow` held-follow runner: bind once, emit `FollowBurst`, stop
  on runtime-only `FollowEnd`. No new `agent-wait/v0` kind
- Follow lease posture rejects at `now >= lease_expires_at`. First-match
  still uses `now >`

## [0.1.3] - 2026-08-18

First crates.io cut for the library crates. No library API change
is intended.

- Workspace `publish` stays `false`. `waitprims-core`,
  `waitprims-async`, and `waitprims-testkit` opt in with
  `publish = true`; `waitprims-cli` inherits unpublished
- Library crates have docs.rs URLs and crate READMEs
- `make release-check` runs `cargo package --workspace` (packages
  all four workspace crates, including the unpublished CLI;
  publishes none)
- README documents a crates.io pin without a patch version

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
- `docs/releases/vX.Y.Z.md` is the this-cut section of
  `RELEASE_NOTES.md` (signed payload). Checksums cover this tag
  only (no leftover earlier-cut notes or archives)

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

[Unreleased]: https://github.com/3leaps/waitprims/compare/v0.1.3...HEAD
[0.1.3]: https://github.com/3leaps/waitprims/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/3leaps/waitprims/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/3leaps/waitprims/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/3leaps/waitprims/releases/tag/v0.1.0
