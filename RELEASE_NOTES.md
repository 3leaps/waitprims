# Release Notes

> **Purge policy:** this file keeps the **latest 3 releases** in
> reverse chronological order. Older cuts live in `docs/releases/`
> and, until the changelog purge, in `CHANGELOG.md`.
> The signed / GitHub payload is the this-cut extract under
> `docs/releases/`, not this landing page.

---

## v0.2.3 — 2026-10-07

Signed release provenance and publication controls. Library APIs and runtime
behavior remain unchanged.

### Highlights

- Adopt [PDR-0002](docs/decisions/PDR-0002-release-publication.md): GPG-signed
  annotated tags with independently approved primary and exact signing subkey,
  read-only artifact CI, and separate maintainer signing/publication steps.
- Commit public GPG/minisign pins, tagger identity and Decernor text/NDJSON
  anchors. Release keys are staged from the verified tagged commit and must
  match its public files byte-for-byte.
- Bind artifact handoff to the annotated tag object, peeled commit and exact
  successful workflow run/attempt. Compare every signed draft asset with the
  verified local bytes before promotion; support bounded draft-upload retries.
- Verify four local library archives on Cargo 1.88.0 using exact workspace
  patches, without skipping archive verification. Registry dry-runs and uploads
  remain unpatched, separately authorized and ordered core → async → testkit → fs.
- Pin release actions and the SBOM image immutably, checksum the actionlint
  download and move macOS jobs to macos-15. CI holds no signing keys or
  release-creation authority and retains all five native CLI artifact targets.

### Upgrade notes

- Workspace crates move from 0.2.2 to 0.2.3; MSRV stays Rust 1.88.0.
- No library API, runtime source or third-party crate pin changes.
- Public JSON remains exactly the six `agent-wait/v0` message kinds.
- The four library crates remain publishable; `waitprims-cli` stays unpublished.
- Local patched archive success does not prove registry availability or waive
  the unpatched predecessor/index checks. Publication archives need not be
  byte-identical to the locally patched proof archives.
- Historical tags remain untouched. Follow the
  [release verification guide](docs/security/README.md) for public-key approval,
  tag verification and signed-manifest checks.

Full notes: [docs/releases/v0.2.3.md](docs/releases/v0.2.3.md)

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
