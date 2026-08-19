# waitprims

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)
[![Rust: 1.88+](https://img.shields.io/badge/rust-1.88%2B-orange.svg)](https://www.rust-lang.org/)
[![crates.io](https://img.shields.io/crates/v/waitprims-core.svg)](https://crates.io/crates/waitprims-core)
[![docs.rs](https://docs.rs/waitprims-core/badge.svg)](https://docs.rs/waitprims-core)

**Reliable event wait without a daemon.**

waitprims is a library-first wait primitive for agent seats and services. It provides scripted first-match wait and will provide typed observe, deliver, and activate — first-match fan-in when you can block, one bounded poll-cycle when you cannot — and keep those receipts distinct. Provider clients, daemons, and durable timers stay out of this crate.

The problem it retires: every adapter reinventing anchors, deadlines, cancellation, and “we posted, so the agent woke.”

**Lifecycle Phase**: `pre-alpha` | Rust only. The library is the product; the CLI is a thin test/diagnostic wrapper. See [RELEASE_NOTES.md](RELEASE_NOTES.md) for the current cut.

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

Same-instant first-match ties use registration-set order (`TIE_RULE`).
Consumed non-winner observations are restored for a later wait.

Poll-cycle events and cursors are not committed until `poll_cycle_ack`
(`POLL_ACK_RETENTION`). Deferred observations replay in order when
restored. Cancel, bound exhaustion, and a restart before ack must not
silently advance cursors.

## Crates

| Crate | Role |
| ----- | ---- |
| `waitprims-core` | Types, errors, validation, digest and time helpers |
| `waitprims-async` | First-match, poll-cycle, and held-follow runners |
| `waitprims-testkit` | Fake clock and scripted observers |
| `waitprims-cli` | Diagnostic CLI (`waitprims`); not on crates.io |

## Status

This cut enables crates.io for the library crates. After that
publication, depend with:

```toml
waitprims-core = "0.1"
waitprims-async = "0.1"
waitprims-testkit = "0.1"
```

Until then, pin a git tag. The diagnostic CLI is not published. APIs
may still move. See [docs/README.md](docs/README.md#install).

Pinned `contract: agent-wait/v0` at Crucible `f1912957cde19b2b1e7809e430cc28dc417287cc`. See [`schemas/v0/PIN.md`](schemas/v0/PIN.md).

`validate_message` and `validate_raw_documents` are the only contract-admission path. `serde_json::from_str` on public message types is not admission.

## Development

```bash
cargo test
cargo build -p waitprims-cli
```

The required CI gate is `fast` on `ubuntu-latest` (`cargo fmt --check`,
`cargo clippy --workspace --all-targets -- -D warnings`, actionlint,
`cargo test --workspace --locked`, MSRV 1.88.0). Hosted smokes are three
cells (linux-x64, macos-arm64, windows-x64) on every push/PR. arm64 labs
(linux-arm64, windows-arm64) are same-repo only and are not a required
gate. Fork PRs do not run the arm64 cells. See
[`.github/CI.md`](.github/CI.md) and [`docs/README.md`](docs/README.md).

`make precommit` is fmt-check and clippy. `make prepush` and
`make pr-final` add locked tests and version-check.
`make release-check` packages the workspace (`cargo package --workspace`)
and publishes none. Only the three library crates are publishable.
See [`RELEASE_CHECKLIST.md`](RELEASE_CHECKLIST.md) for the later signed
GitHub release flow.

The diagnostic binary lands at `target/debug/waitprims`.

```bash
waitprims --help
waitprims --version
waitprims validate --input <file-or-directory>
waitprims wait --registration-set <file> --request <file> --script <file>
waitprims poll --registration-set <file> --request <file> --script <file>
waitprims schema [--message-type <message_type>]
```

`validate --input` admits one message file or a directory set. `wait` resolves a cited registration set and first-matches a scripted observer. `poll` runs one bounded poll-cycle over the same set. `schema` prints the bundled JSON Schema (`$id` and document); `--message-type` prints that kind's definition without minting a fragment `$id`. JSON goes to stdout. Logs and errors go to stderr. Scripts are local files; `--script -` is rejected.

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Subject to [3 Leaps OSS policies](https://github.com/3leaps/oss-policies),
including [Sensitive Local Data](https://github.com/3leaps/oss-policies/blob/main/SENSITIVE-LOCAL-DATA.md).
