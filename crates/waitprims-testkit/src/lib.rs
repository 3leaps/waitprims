//! Deterministic test helpers for waitprims.
//!
//! Fake clocks and scripted observers live here. This crate does not
//! perform network I/O.

mod bind;
mod case;
mod clock;
mod observer;
mod script;
mod seam;

pub use bind::{exclusive_head_anchor, resolve_start_at_bind, BindTracker, TrackedBind};
pub use case::{
    ack_poll_outcome, arm_id_for, live_wait_request, poll_cycle_request, registration,
    registration_baseline, registration_set, ts, wait_event,
};
pub use clock::FakeClock;
pub use observer::{EndlessReadyObserver, IdleObserver, ScriptedObserver};
pub use script::Script;
pub use seam::ScriptedReceipts;
pub use waitprims_async::{
    event_surface_bytes, run_poll_cycle, BindHandle, Cancel, Clock, Error, Observation, Observer,
    Result, POLL_ACK_RETENTION, TIE_RULE,
};

#[cfg(test)]
mod platform_proofs;
#[cfg(test)]
mod poll_proofs;
#[cfg(test)]
mod proofs;
#[cfg(test)]
mod receipt_proofs;
