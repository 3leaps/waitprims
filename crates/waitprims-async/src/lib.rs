//! Async first-match, poll-cycle, held-follow, and coalesce runners.
//!
//! Tokio is used for runtime, time, and synchronization only. This crate
//! does not open network sockets and is not a daemon.
//!
//! Public JSON remains exactly the six `agent-wait/v0` message kinds.
//! The observer seam (bind / next / cancel) is a runtime interface, not a
//! wire type. [`FollowBurst`], [`FollowEnd`], [`CoalesceBurst`], and
//! [`CoalescePolicy`] are runtime-only. `priority` is a presentation
//! hint, not authorization.
//! Delivery and activation stay caller-owned or opaque refs;
//! runners do not collapse them into match.
//!
//! [`Cancel`] is a portable watch token. Deadlines use [`Clock`], not
//! `EINTR`, unix-domain sockets, signals, or a Windows Job Object.
//! [`Observer::restore_ready`] errors fail closed: runners return `Err`
//! rather than drop a consumed observation.

mod cancel;
mod clock;
mod coalesce;
mod first_match;
mod follow;
mod observer;
mod outcome;
mod poll_cycle;
mod race;

pub use cancel::Cancel;
pub use clock::Clock;
pub use coalesce::{run_coalesce, CoalesceBurst, CoalescePolicy};
pub use first_match::{run_first_match, TIE_RULE};
pub use follow::{run_follow, FollowBurst, FollowEnd, TerminalArmKind};
pub use observer::{BindHandle, Observation, Observer};
pub use poll_cycle::{event_surface_bytes, run_poll_cycle, POLL_ACK_RETENTION};
pub use waitprims_core::{Error, Result};

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn runtime_is_available() {
        tokio::time::sleep(std::time::Duration::from_millis(0)).await;
    }
}
