---
id: PDR-0002
title: Signed tags, read-only artifact CI and maintainer publication
status: accepted
date: 2026-10-05
scope: waitprims release process
---

# PDR-0002: Release publication

## Decision

Release tags are GPG-signed annotated tags with an independently approved
primary and exact signing subkey. Public key pin, tagger identity and
Decernor-derived text/NDJSON anchors are committed and reviewed before the cut.

Tag CI is read-only and preserves the five native build targets. It uploads
workflow artifacts with annotated tag-object and peeled-commit identity. The
maintainer's trusted checkout selects the exact successful run/attempt,
re-verifies the approved signed remote tag, stages the complete asset set,
creates a local draft and signs checksum manifests with minisign. Upload and
promotion are separate guarded maintainer operations.

Prepublication local archive verification on Cargo 1.88 uses exact workspace
source patches. Registry operations use no patches and proceed core, async,
testkit, fs, with indexed predecessor confirmation after each separately cued
upload. The CLI remains unpublished. A previous core version does not satisfy
new-version internal requirements. Tag CI completion does not require a prior
registry upload. The trusted operator checkout retains registry orchestration;
Cargo uses a separate verified-source worktree. Registry calls require the
original recorded ceremony object and commit across separate operations.

Post-tag gates read public material from the tagged commit as inert Git
objects and execute trusted checkout code. They remain independent of newer
main VERSION changes. Independent primary approval is required even when a
candidate key and its own anchors agree. Every publication check binds both
the tag object and commit. Promotion also compares every remote asset's bytes
with the verified local signed set. A draft with a partial or completed subset
of approved names may be restored by re-uploading the exact local set; changed
tag/target or unexpected names fail. Post-tag PGP selectors read released public
material rather than newer operator anchors; mutable-ref races between checks and API calls
remain a residual. Repository defaults and tag rulesets are checked separately;
read-only workflow YAML does not establish an external permission ceiling.

## Scope

Four Rust libraries, five native CLI archive targets, MSRV 1.88.0 and the
existing wire contract. No signing or registry credentials in CI. Historical
released tags remain unchanged. Public-anchor rotation is reviewed separately.

This process supersedes the CI-draft/package-index ordering in PDR-0001.
The previous record remains historical. Acceptance requires maintainer review.
