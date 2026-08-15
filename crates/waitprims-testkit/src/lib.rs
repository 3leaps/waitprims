//! Deterministic test helpers for waitprims.
//!
//! Fake clocks and scripted observers live here. This crate does not
//! perform network I/O.

pub use waitprims_async::{Error, Result};

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn crate_compiles() {}
}
