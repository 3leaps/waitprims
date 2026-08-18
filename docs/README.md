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
`f1912957cde19b2b1e7809e430cc28dc417287cc`.

See [`schemas/v0/PIN.md`](../schemas/v0/PIN.md). The vendored
`contract.json` plus `agent-wait-message.schema.json` are the L2 entry.
Do not invent a parallel schema in this tree.

## FakeClock

`waitprims-testkit` `FakeClock` is logical. Same-instant ties, restore,
and poll-ack use contract timestamps and registration-set order. Do not
key uniqueness on a wall `sleep`. Windows timer granularity is coarser
than Unix 1ms; contract timestamps stay distinct.

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
| `waitprims-async` | First-match and poll-cycle runners |
| `waitprims-testkit` | Fake clock and scripted observers |
| `waitprims-cli` | Diagnostic binary (`waitprims`) |

## Releases

Latest three cuts: [`RELEASE_NOTES.md`](../RELEASE_NOTES.md).
Per-version notes live in [`releases/`](releases/). The signed-release
flow is [`RELEASE_CHECKLIST.md`](../RELEASE_CHECKLIST.md).
