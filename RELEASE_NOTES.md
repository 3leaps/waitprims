# Release Notes

> **Purge policy:** this file keeps the **latest 3 releases** in
> reverse chronological order. Older cuts live in `docs/releases/`
> and, until the changelog purge, in `CHANGELOG.md`.
> The signed / GitHub payload is the this-cut extract under
> `docs/releases/`, not this landing page.

---

## v0.2.2 — 2026-09-01

Native local-filesystem observation and a diagnostic command that exercises
it directly from a checkout.

### Highlights

- New `waitprims-fs` implements the existing `Observer` contract with the
  platform-native `RecommendedWatcher`; it does not add a polling fallback.
- Native create, write, remove, and rename notifications become minimal,
  root-relative descriptors materialized through a caller-owned payload sink.
- Filesystem binds fail closed on unsupported posture, unsafe path changes,
  rescan requirements, ambiguous events for specific predicates, overflow,
  and sink or digest failures.
- The observer works with the existing first-match, poll-cycle, held-follow,
  and held-coalesce runners without changing the six public wire messages.
- Diagnostic `waitprims watch` accepts local registration and request files,
  runs one native filesystem source, and emits the existing
  `follow_burst` / `follow_end` JSONL views.
- Native demo coverage runs across the supported CI platform matrix without
  introducing a public watcher API or filesystem polling.

### Upgrade notes

- Workspace crates move from 0.2.1 to 0.2.2.
- `waitprims-fs` is newly available as a library crate.
- `waitprims-cli` remains unpublished.
- Public JSON remains exactly the six `agent-wait/v0` message kinds.
- Git users should pin `v0.2.2`.

Full notes: [docs/releases/v0.2.2.md](docs/releases/v0.2.2.md)

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
