# waitprims-async

Async first-match, poll-cycle, held-follow, and coalesce runners for waitprims.

Tokio is used for runtime, time, and synchronization only. This crate
does not open network sockets and is not a daemon.

```toml
waitprims-async = "0.2"
```

Depends on [`waitprims-core`](https://crates.io/crates/waitprims-core).
Docs: <https://docs.rs/waitprims-async>

Licensed under MIT OR Apache-2.0. See the [waitprims](https://github.com/3leaps/waitprims)
repository.
