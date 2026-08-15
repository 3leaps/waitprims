//! Core types, errors, and helpers for waitprims.
//!
//! Public JSON is exactly the six `agent-wait/v0` message kinds. Runtime-only
//! types must not serialize as that contract.

pub mod digest;
pub mod error;
pub mod jcs;

pub use error::{Error, Result};
pub use time::OffsetDateTime;

#[cfg(test)]
mod tests {
    #[test]
    fn crate_compiles() {}
}
