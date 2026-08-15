//! Logical clock used by the first-match runner.

use std::future::Future;

use waitprims_core::Timestamp;

/// Source of logical time for waits and backoff.
///
/// Implementations must not open sockets. Test clocks live in
/// `waitprims-testkit`.
pub trait Clock: Send + Sync {
    /// Current logical time.
    fn now(&self) -> Timestamp;

    /// Wait until `deadline` in this clock's time base.
    fn sleep_until(&self, deadline: &Timestamp) -> impl Future<Output = ()> + Send;
}
