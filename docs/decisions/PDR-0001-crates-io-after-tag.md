---
id: "PDR-0001"
title: "crates.io after the git tag, before the GitHub Release is green"
status: "accepted"
date: "2026-08-18"
last_updated: "2026-08-18"
deciders:
  - "@3leapsdave"
scope: "waitprims release process"
tags:
  - "process"
  - "release"
  - "crates-io"
relates-to:
  - "RELEASE_CHECKLIST.md"
  - "3leaps/crucible ADR-0003 (*DR taxonomy; PDR = revisable process)"
---

# PDR-0001: crates.io after the git tag

## Status

**Accepted.** Recorded from the v0.1.3 cut. Implemented in
`RELEASE_CHECKLIST.md` in the same change.

## Context

waitprims is a library-first workspace. Three crates opt in to
crates.io (`waitprims-core`, `waitprims-async`, `waitprims-testkit`).
The diagnostic CLI stays unpublished. Dependents use `version` +
`path` so `cargo publish` rewrites them to registry versions.

MSRV is 1.88. On that Cargo, `cargo package --workspace` looks up
path dependents on crates.io when preparing the next crate. It
does **not** use the later tmp-registry behaviour of Cargo 1.90.
So the tag Release workflow's Package Check fails for a **new**
version until `waitprims-core@VERSION` exists on the index.

v0.1.3 hit that: the git tag CI was green; Release Package Check
failed; there was no draft to sign until the three libraries were
published and the workflow re-run.

The checklist used to put crates.io **after** MFA sign and undraft.
That order cannot produce a signable GitHub draft on first upload
(and is unnecessary delay on later cuts).

## Decision

1. **Order.** `cargo publish` the three library crates **after**
   the annotated git tag is on `origin`, **before** treating the
   GitHub Release workflow as green. Then MFA-sign and undraft.
   Later cuts may see Package Check pass without a new publish
   (previous core is already on the index); still publish this
   tag's `VERSION` after the tag exists so git and crates.io match.

2. **No backfill.** First registry version of a crate name is the
   cut that enabled `publish = true`. Older git tags stay off
   crates.io.

3. **Tokens (OOB).** Use the crates.io account that will own the
   3leaps crates. Do not reuse another org's token.

   | Token role | Scopes | Use |
   | ---------- | ------ | --- |
   | new + update | `publish-new`, `publish-update` | first upload of a crate name |
   | update only | `publish-update` | later versions of existing crates |

   Restrict both tokens to the three library crate names (add other
   3leaps crate names when those cuts are scheduled). No crate-name
   wildcard exists. No `yank` unless a separate playbook says so.
   Expiry 30–90 days. Store OOB as `CARGO_REGISTRY_TOKEN_3LEAPS`
   (and a `_NEW` sibling if you keep both). Never commit a token.

4. **Index proof.** After each upload, wait on
   `cargo info --registry crates-io <crate>@VERSION`. Bare
   `cargo info` can resolve the workspace and is not an index proof.

5. **CLI.** Never `cargo publish -p waitprims-cli`.

This is a **PDR** (process), not an ADR. Record types follow
[crucible ADR-0003](https://github.com/3leaps/crucible/blob/main/docs/decisions/ADR-0003-decision-record-taxonomy.md).

## Consequences

- First-cut and MSRV Package Check become unblocked without
  raising the Release workflow's Rust toolchain.
- Two tokens limit blast: day-to-day update cannot mint a new
  crate name.
- A missed crates.io step still fails the tag Release job on
  first upload — that is the signal, not a checklist-only hope.

## Revision History

| Date       | Status Change | Summary                                      | Updated By |
| ---------- | ------------- | -------------------------------------------- | ---------- |
| 2026-08-18 | → accepted    | Tag, then crates.io, then GitHub sign/undraft | echo-devlead |
