//! Async first-match and poll-cycle runners.
//!
//! Tokio is used for runtime, time, and synchronization only. This crate
//! does not open network sockets.
//!
//! Public JSON remains exactly the six `agent-wait/v0` message kinds.
//! The observer seam (bind / next / cancel) is a runtime interface, not a
//! wire type.

mod cancel;
mod clock;
mod first_match;
mod observer;
mod outcome;
mod poll_cycle;
mod race;

pub use cancel::Cancel;
pub use clock::Clock;
pub use first_match::{run_first_match, TIE_RULE};
pub use observer::{BindHandle, Observation, Observer};
pub use poll_cycle::run_poll_cycle;
pub use waitprims_core::{Error, Result};

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn runtime_is_available() {
        tokio::time::sleep(std::time::Duration::from_millis(0)).await;
    }
}
