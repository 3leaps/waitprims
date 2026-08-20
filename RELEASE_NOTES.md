# Release Notes

> **Purge policy:** this file keeps the **latest 3 releases** in
> reverse chronological order. Older cuts live in `docs/releases/`
> and, until the changelog purge, in `CHANGELOG.md`.
> The signed / GitHub payload is the this-cut extract under
> `docs/releases/`, not this landing page.

---

## v0.2.0 — unreleased

Local cut. Source-breaking optional `registration.priority` (inert).
Not a 0.1.x patch. Presentation hint, not authorization. No public tag yet.

---

## v0.1.3 — 2026-08-18

First crates.io cut for the library crates. No library API change
is intended.

### Highlights

- `waitprims-core`, `waitprims-async`, and `waitprims-testkit` are
  publishable. Workspace `publish` stays `false`; those three crates
  opt in. Each has a README and a docs.rs URL.
- `waitprims-cli` stays unpublished (diagnostic).
- `make release-check` runs `cargo package --workspace`. That
  packages all four workspace crates, including the unpublished
  CLI, and does not publish.

### Upgrade notes

- No public API change.
- Depend on the library crates from crates.io
  (`waitprims-async = "0.1"`). A git tag pin still works.
- Signing is local MFA (`make release-sign` / `make release`). CI
  still drafts an unsigned GitHub release.

Full notes: [docs/releases/v0.1.3.md](docs/releases/v0.1.3.md)

---

## v0.1.2 — 2026-08-18

Release-kit hygiene. No library API change is intended. This release
does not publish to crates.io or change repository visibility.

### Highlights

- Makefile release leaf targets have no write-chain precursors.
  `make release-export-keys` does not re-clean or re-download.
- `make release` is the only serialized walk: clean, download, notes,
  checksums, sign, export-keys, upload. Verification runs once, as
  the upload grouping prerequisite.
- CLI `--version` test tracks the workspace `VERSION` file.
- Makefile `precommit` and `pr-final` sit beside `prepush`.

### Upgrade notes

- No public API change.
- `publish` stays `false`. Pin from git if you already pin `v0.1.1`.
- Signing is local MFA (`make release-sign` / `make release`). CI
  still drafts an unsigned GitHub release.

Full notes: [docs/releases/v0.1.2.md](docs/releases/v0.1.2.md)

---

## v0.1.1 — 2026-08-18

Signed-release kit. No library API change is intended.

### Highlights

- `VERSION` is the version SSOT (`make version-sync`, `make version-check`).
- Makefile release targets and `scripts/` for download, checksums, MFA
  sign, verify, and upload.
- Draft GitHub release workflow on `v*` tags: CLI archives for the five
  smoked platforms, SBOM, `LICENSE-MIT`, `LICENSE-APACHE`.
- `make release-check` packages the workspace and does not run
  `cargo publish`. `publish` stays `false`.

Full notes: [docs/releases/v0.1.1.md](docs/releases/v0.1.1.md)
