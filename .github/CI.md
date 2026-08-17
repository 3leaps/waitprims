# CI note

The required PR gate is `fast` on `ubuntu-latest`. The five native cells
are not one required 5-wide gate. `platform-smoke-arm64` is same-repo
only and must not be marked required.

## Jobs

| Job | When | Role |
| --- | --- | --- |
| `fast` | every push/PR | fmt, clippy `-D warnings`, `cargo test --workspace --locked`, MSRV 1.88.0 |
| `platform-smoke-hosted` | every push/PR | native clippy + locked tests; `fail-fast: false` |
| `platform-smoke-arm64` | same-repo only | native clippy + locked tests; `fail-fast: false`; not a required gate |

`.github/actionlint.yaml` lists the custom runner labels so actionlint
does not treat them as unknown. That file does not deploy a runner.

## Five cells (native, no cross)

| Job | `runs-on` | rustc host |
| --- | --- | --- |
| hosted | `ubuntu-latest` | `x86_64-unknown-linux-gnu` |
| hosted | `macos-14` | `aarch64-apple-darwin` |
| hosted | `windows-latest` | `x86_64-pc-windows-msvc` |
| arm64 | `ubuntu-latest-arm64-s` | `aarch64-unknown-linux-gnu` |
| arm64 | `windows-latest-arm64-s` | `aarch64-pc-windows-msvc` |

No musl, no macOS Intel, no `goneat-tools-runner`. Windows steps use
`shell: bash`.

## Clock and POSIX gaps

`waitprims-testkit` `FakeClock` is logical. Same-instant ties, restore,
and poll-ack use contract timestamps and registration-set order. Do not
add sleep-based uniqueness tests that assume Unix 1ms timers. Windows
timer granularity is coarser.

Windows has no `EINTR`, unix domain sockets, or signal-driven cancel.
`Cancel` is a portable watch token. `#[cfg(unix)]` is only for real
POSIX-only paths; cancel/timeout have a portable Windows counterpart.

Pin stays `contract: agent-wait/v0` at
`f1912957cde19b2b1e7809e430cc28dc417287cc`.
