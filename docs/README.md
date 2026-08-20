# waitprims

Library-first wait primitive. No daemon, no provider SDK, no durable timer
ledger. The CLI is a thin diagnostic wrapper.

Public JSON is exactly the six `agent-wait/v0` message kinds. There is no
`WaitSpec`, `live_wait_ack`, or tenth kind.

## Six message kinds

| Kind | Role |
| ---- | ---- |
| `registration_set` | Snapshot of registrations for one waiter/seat |
| `live_wait_request` | First-match wait when the caller can block |
| `live_wait_outcome` | First-match result |
| `poll_cycle_request` | One bounded poll-cycle when the caller cannot block |
| `poll_cycle_outcome` | Poll-cycle result (uncommitted until ack) |
| `poll_cycle_ack` | Consumer commit of per-registration cursors |

Admission is `validate_message` / `validate_raw_documents` only.
`serde_json::from_str` on a public type is not admission.

## Crucible pin

`contract: agent-wait/v0` at
`4bc95146bc4ed503180fb13971947854a36957cb` (crucible `v0.1.28`).
Optional `registration.priority` is a cooperative presentation hint,
not authorization, grant, quota, or abort.

See [`schemas/v0/PIN.md`](../schemas/v0/PIN.md). The vendored
`contract.json` plus `agent-wait-message.schema.json` are the L2 entry.
Do not invent a parallel schema in this tree.

## FakeClock

`waitprims-testkit` `FakeClock` is logical. Same-instant ties, restore,
and poll-ack use contract timestamps and registration-set order. Do not
key uniqueness on a wall `sleep`. Windows timer granularity is coarser
than Unix 1ms; contract timestamps stay distinct.

## Held follow

`run_follow` binds once per registration and emits runtime-only
`FollowBurst` values until `FollowEnd` (cancel, deadline, or a
fail-closed arm). It is not a seventh `agent-wait/v0` kind.
`run_follow` does not read `priority`.

`run_coalesce` is a second held-session runner on the same binds. It
emits runtime-only `CoalesceBurst` values under `CoalescePolicy`
(`min_emit_interval`, `urgent_at`). Quiet events wait for the timer;
`priority >= urgent_at` (omitted = 50) flushes immediately. `FollowEnd`
is reused. `priority` is not authorization.

## Portable cancel

`Cancel` is a cloneable watch token. Deadlines use `Clock`, not `EINTR`,
unix-domain sockets, signals, or a Windows Job Object. The portable path
is the only claimed path.

## Schema-backed interactions

Interactions are the six kinds above, validated against the pinned
schema. Point callers at [`schemas/v0/`](../schemas/v0/). The schema
`$id` is not the contract-entry mechanism; resolve `capability` through
`contract.json`, then load the relative `entry_schema`.

## Crates

| Crate | Role |
| ----- | ---- |
| `waitprims-core` | Types, errors, validation, digest and RFC3339 helpers |
| `waitprims-async` | First-match, poll-cycle, and held-follow runners |
| `waitprims-testkit` | Fake clock and scripted observers |
| `waitprims-cli` | Diagnostic binary (`waitprims`); not on crates.io |

## Install

This cut enables crates.io for the three library crates. After that
publication, pin the minor line; do not copy a patch from this file.

```toml
waitprims-core = "0.1"
waitprims-async = "0.1"
waitprims-testkit = "0.1"
```

Until the crates are on the registry, pin a git tag. The CLI is
diagnostic only. From the repository root, build it:

```bash
cargo build --locked -p waitprims-cli
```

The binary is `./target/debug/waitprims`. Or install it:

```bash
cargo install --path crates/waitprims-cli --locked --force
```

It is not a crates.io package. `waitprims follow` streams diagnostic
JSONL (`diagnostic_type`, not `message_type`) for the held-follow
runner. `waitprims contract` prints the compiled pin.

Canonical held-follow demo (repository root; after a locked fetch or
build so the cargo cache is present):

```bash
make demo-follow
```

That builds with `cargo build --locked --offline -p waitprims-cli` and
compares stdout to
[`fixtures/follow-demo/golden.jsonl`](../fixtures/follow-demo/golden.jsonl).
See [`fixtures/follow-demo/README.md`](../fixtures/follow-demo/README.md)
for the copyable three-file command.

docs.rs pages appear after the first crates.io upload:
[waitprims-core](https://docs.rs/waitprims-core),
[waitprims-async](https://docs.rs/waitprims-async),
[waitprims-testkit](https://docs.rs/waitprims-testkit).

## Releases

Latest three cuts: [`RELEASE_NOTES.md`](../RELEASE_NOTES.md)
(purge: three). Older cuts stay in [`releases/`](releases/)
(including [`v0.1.0`](releases/v0.1.0.md)). `CHANGELOG.md` keeps
the latest ten (not yet over that). The signed-release flow is
[`RELEASE_CHECKLIST.md`](../RELEASE_CHECKLIST.md).
Process records live in [`decisions/`](decisions/).
