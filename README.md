# waitprims

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)
[![Rust: 1.88+](https://img.shields.io/badge/rust-1.88%2B-orange.svg)](https://www.rust-lang.org/)

**Reliable event wait without a daemon.**

waitprims is a library-first wait primitive for agent seats and services. It provides scripted first-match wait and will provide typed observe, deliver, and activate — first-match fan-in when you can block, one bounded poll-cycle when you cannot — and keep those receipts distinct. Provider clients, daemons, and durable timers stay out of this crate.

The problem it retires: every adapter reinventing anchors, deadlines, cancellation, and “we posted, so the agent woke.”

**Lifecycle Phase**: `pre-alpha` | v0.1.x is Rust only. The library is the product; the CLI is a thin test/diagnostic wrapper.

## What this is

- Targets Crucible `contract: agent-wait/v0` (see Status)
- MIT OR Apache-2.0
- No daemon, no provider SDKs, no durable timer ledger
- Adapters live with their domain owner, not in this library

## What this is not

- Not GNU timeout (that is sysprims)
- Not a chat, SMS, or job-plane client
- Not a webhook, MCP, or daemon host

## Public JSON

Public JSON is exactly the six `agent-wait/v0` messages:

- `registration_set`
- `live_wait_request`
- `live_wait_outcome`
- `poll_cycle_request`
- `poll_cycle_outcome`
- `poll_cycle_ack`

There is no public `WaitSpec`, `live_wait_ack`, delivery message, or
activation message. Delivery and activation stay off this wire as opaque
refs when a caller uses them.

## Crates

| Crate | Role |
| ----- | ---- |
| `waitprims-core` | Types, errors, validation, digest and time helpers |
| `waitprims-async` | First-match and poll-cycle runners |
| `waitprims-testkit` | Fake clock and scripted observers |
| `waitprims-cli` | Diagnostic CLI (`waitprims`) |

## Status

This tree is an early scaffold (`0.1.0-dev`). APIs are not stable.

Pinned `contract: agent-wait/v0` at Crucible `f1912957cde19b2b1e7809e430cc28dc417287cc`. See [`schemas/v0/PIN.md`](schemas/v0/PIN.md).

`validate_message` and `validate_raw_documents` are the only contract-admission path. `serde_json::from_str` on public message types is not admission.

## Development

```bash
cargo test
cargo build -p waitprims-cli
```

The diagnostic binary lands at `target/debug/waitprims`.

```bash
waitprims --help
waitprims --version
waitprims validate --input <file-or-directory>
waitprims wait --registration-set <file> --request <file> --script <file>
waitprims poll --registration-set <file> --request <file> --script <file>
```

`validate --input` admits one message file or a directory set. `wait` resolves a cited registration set and first-matches a scripted observer. `poll` runs one bounded poll-cycle over the same set. JSON goes to stdout. Logs and errors go to stderr.

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
