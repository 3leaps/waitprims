# Release Notes

> **Purge policy:** this file keeps the **latest 3 releases** in
> reverse chronological order. Older cuts live in `docs/releases/`
> and, until the changelog purge, in `CHANGELOG.md`.
> The signed / GitHub payload is the this-cut extract under
> `docs/releases/`, not this landing page.

---

## v0.2.1 — 2026-08-27

Held-follow and held-coalesce runners, deterministic diagnostic demos,
and the updated `agent-wait/v0` contract pin.

There is no v0.2.0 tag: its inert priority field landed in the same
squash commit as v0.2.1 and is included in this release.

### Highlights

- `run_follow` binds once and emits runtime-only `FollowBurst` values
  until a runtime-only `FollowEnd`.
- `run_coalesce` adds quiet-window coalescing and priority-triggered
  emission while preserving Observer custody and backpressure.
- Optional `registration.priority` is a presentation hint, not
  authorization. Omitted and explicit 50 remain digest-distinct.
- Diagnostic `follow` and `coalesce` commands use local scripted
  observers and emit diagnostic JSONL without adding a public wire kind.
- Coalescing proofs cover overflow custody, sink failures, ordering,
  quiet liveness, and held-bind backpressure.
- The pinned `agent-wait/v0` contract is Crucible `v0.1.28`
  (`4bc95146…`).

### Upgrade notes

- The three library crates move from 0.1.3 to 0.2.1.
- `waitprims-cli` remains unpublished.
- Public JSON remains exactly the six `agent-wait/v0` message kinds.
- Git users should pin `v0.2.1`; there is no `v0.2.0` tag.

Full notes: [docs/releases/v0.2.1.md](docs/releases/v0.2.1.md)

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
