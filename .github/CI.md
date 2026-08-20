# CI note

The required PR gate is `fast` on `ubuntu-latest`. Hosted smokes are
three cells on every push/PR. arm64 labs are two more cells, same-repo
only, and must not be marked required. Fork PRs therefore see three
native smokes, not five.

## Jobs

| Job | When | Role |
| --- | --- | --- |
| `fast` | every push/PR | fmt, clippy `-D warnings`, actionlint, `cargo test --workspace --locked`, `make demo-follow`, MSRV 1.88.0 |
| `platform-smoke-hosted` | every push/PR | three native cells; clippy + locked tests; `fail-fast: false` |
| `platform-smoke-arm64` | same-repo only | two native cells; clippy + locked tests; `fail-fast: false`; not a required gate |

`.github/actionlint.yaml` lists the custom runner labels so actionlint
does not treat them as unknown. That file does not deploy a runner.

## Hosted smokes (every push/PR)

| `runs-on` | rustc host |
| --- | --- |
| `ubuntu-latest` | `x86_64-unknown-linux-gnu` |
| `macos-14` | `aarch64-apple-darwin` |
| `windows-latest` | `x86_64-pc-windows-msvc` |

## arm64 labs (same-repo only)

| `runs-on` | rustc host |
| --- | --- |
| `ubuntu-latest-arm64-s` | `aarch64-unknown-linux-gnu` |
| `windows-latest-arm64-s` | `aarch64-pc-windows-msvc` |

No musl, no macOS Intel, no `goneat-tools-runner`. Windows steps use
`shell: bash`. The five native cells exist as an estate; they are not
one required 5-wide gate, and fork PRs do not run the arm64 pair.

## Clock and POSIX gaps

`waitprims-testkit` `FakeClock` is logical. Same-instant ties, restore,
and poll-ack use contract timestamps and registration-set order. Do not
add sleep-based uniqueness tests that assume Unix 1ms timers. Windows
timer granularity is coarser.

Windows has no `EINTR`, unix domain sockets, or signal-driven cancel.
`Cancel` is a portable watch token. There is no Job Object claim.
`#[cfg(unix)]` is only for real POSIX-only paths; cancel/timeout have a
portable Windows counterpart.

Pin stays `contract: agent-wait/v0` at
`f1912957cde19b2b1e7809e430cc28dc417287cc`.
