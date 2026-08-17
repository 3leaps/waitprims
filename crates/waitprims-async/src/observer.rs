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
///
/// An observation is a candidate (payload by ref) or an arm signal. It is
/// not a wait result and is not an `agent-wait/v0` message kind.
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
    /// Provider outage on this arm. Not a clean `no_change` / `logical_deadman`.
    Outage {
        /// Stable reason code for the outage arm.
        reason_code: waitprims_core::IdToken,
    },
    /// The exclusive cursor is uncertain. Not a clean complete.
    CursorUncertain {
        /// Stable reason code for the uncertain arm.
        reason_code: waitprims_core::IdToken,
    },
    /// The arm is degraded. Not a clean `no_change` / `logical_deadman`.
    Degraded {
        /// Stable reason code for the degraded arm.
        reason_code: waitprims_core::IdToken,
    },
}

impl Observation {
    /// An observation is never itself a wait result.
    pub const fn is_wait_result(&self) -> bool {
        false
    }
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
    ///
    /// A consuming implementation must pair every dequeue with
    /// [`Self::restore_ready`]. There is no silent drop path.
    fn poll_ready(&self, _bind: &Self::Bind) -> Option<Observation> {
        None
    }

    /// Put a consumed observation back so a later cycle can replay it.
    ///
    /// Required whenever [`Self::next`] or [`Self::poll_ready`] transferred
    /// an owned replayable observation (typically [`Observation::Event`]).
    /// A no-op is valid only when those methods never dequeue such an
    /// observation (for example a synthetic [`Observation::Idle`]).
    /// Default [`Self::poll_ready`] does not excuse a no-op for values
    /// taken from [`Self::next`].
    ///
    /// Returns `Err` when requeue fails. Runners fail closed on that error
    /// rather than dropping the observation.
    fn restore_ready(&self, bind: &Self::Bind, obs: Observation) -> Result<()>;
}
