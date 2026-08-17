# CI note

waitprims is a waiter. The five-platform matrix exists because a
linux-x86-only job would hide the races this crate is supposed to catch.

## Clock resolution

Windows default timer granularity is typically about 15.6 ms. macOS and
Linux are finer. Same-instant first-match ties, loser restore, and
poll-ack commit must use contract timestamps and registration-set order.
They must not use a sub-millisecond `sleep` or a wall `Instant` as a
uniqueness key. A 1 ms sleep that distinguishes two events on Linux can
collapse on Windows.

## POSIX gaps on Windows

Windows has no `EINTR`, no unix domain sockets, and no signal-driven
cancel. `Cancel` is a portable watch token. Deadlines go through
`Clock::sleep_until`. Unix-only paths, if any, stay behind `#[cfg(unix)]`
and have a Windows counterpart on this portable cancel/timeout path.

## Estate runner labels

| Platform        | `runs-on`                |
| --------------- | ------------------------ |
| linux x86_64    | `ubuntu-latest`          |
| linux arm64     | `ubuntu-latest-arm64-s`  |
| windows x86_64  | `windows-latest`         |
| windows arm64   | `windows-latest-arm64-s` |
| macos aarch64   | `macos-14`               |

`fail-fast: false` so one platform does not hide another. rustc is
workspace MSRV 1.88.0. Pin stays `contract: agent-wait/v0` at
`f1912957cde19b2b1e7809e430cc28dc417287cc`.
