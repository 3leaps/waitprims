//! Observer seam: bind / next / cancel.
//!
//! This is a runtime interface, not an `agent-wait/v0` message kind.

use std::future::Future;

use waitprims_core::{Anchor, IdToken, Registration, Result, WaitEvent};

/// Handle returned by [`Observer::bind`].
///
/// Dropping the handle is the release guarantee. `resolved_start` is the
/// exclusive provider cursor assigned at bind — including when the
/// registration cited `baseline_policy` rather than a cursor.
pub trait BindHandle: Send + Sync + Unpin {
    /// Registration this bind observes.
    fn registration_id(&self) -> &IdToken;

    /// Exclusive start cursor resolved at bind. Never a policy label.
    ///
    /// Every successful bind must expose this cursor. There is no optional
    /// or pending start after `bind()` returns.
    fn resolved_start(&self) -> &Anchor;
}

/// One observation from a bound registration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observation {
    /// An accepted source event.
    Event(Box<WaitEvent>),
    /// Nothing is ready; the runner may back off.
    Idle,
    /// The observer's bounded buffer overflowed. Typed, not silent.
    Overflow,
    /// The arm failed without producing a usable event.
    Failed {
        /// Stable reason code for a `failed` outcome.
        reason_code: waitprims_core::IdToken,
    },
}

/// Observe registrations until the wait completes or the bind is dropped.
///
/// Shape is not frozen; bind / next / cancel is the required seam.
/// [`Self::Bind`] must expose the exclusive start cursor resolved at bind.
pub trait Observer: Send + Sync {
    /// Handle returned by [`Self::bind`]. Dropping it must release the bind.
    type Bind: BindHandle;

    /// Bind one registration. Failure prevents a valid outcome.
    fn bind(&self, registration: &Registration) -> impl Future<Output = Result<Self::Bind>> + Send;

    /// Wait for the next observation on a bind.
    fn next(&self, bind: &Self::Bind) -> impl Future<Output = Result<Observation>> + Send;

    /// Best-effort explicit release. Must be idempotent.
    ///
    /// The first-match runner does not await this after a decision. A hung
    /// cancel must not delay a decided outcome. Drop of [`Self::Bind`] is
    /// the release guarantee.
    fn cancel(&self, bind: &Self::Bind) -> impl Future<Output = Result<()>> + Send;

    /// Non-blocking check for an observation that is already due.
    ///
    /// Used to collect same-instant ties and to prefer a ready event over a
    /// deadman when both fire at the same logical time. Default: none.
    fn poll_ready(&self, _bind: &Self::Bind) -> Option<Observation> {
        None
    }
}
