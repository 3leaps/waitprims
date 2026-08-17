//! Cooperative cancellation for a live wait.

use tokio::sync::watch;

/// Signaled cancellation for [`crate::run_first_match`].
///
/// The token is cloneable. Triggering any clone cancels waiters on every clone.
#[derive(Clone, Debug)]
pub struct Cancel {
    tx: watch::Sender<bool>,
    rx: watch::Receiver<bool>,
}

impl Cancel {
    /// A token that is not yet cancelled.
    pub fn new() -> Self {
        let (tx, rx) = watch::channel(false);
        Self { tx, rx }
    }

    /// Signal cancellation.
    pub fn trigger(&self) {
        let _ = self.tx.send(true);
    }

    /// Whether cancellation has already been signaled.
    pub fn is_cancelled(&self) -> bool {
        *self.rx.borrow()
    }

    /// Resolve when cancellation is signaled.
    pub async fn cancelled(&self) {
        if self.is_cancelled() {
            return;
        }
        let mut rx = self.rx.clone();
        while rx.changed().await.is_ok() {
            if *rx.borrow() {
                return;
            }
        }
    }
}

impl Default for Cancel {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::Cancel;

    async fn prove_portable_cancel_wakes_without_eintr_or_signal() {
        let cancel = Cancel::new();
        let waiter = {
            let cancel = cancel.clone();
            tokio::spawn(async move {
                cancel.cancelled().await;
            })
        };
        tokio::task::yield_now().await;
        cancel.trigger();
        tokio::time::timeout(Duration::from_secs(2), waiter)
            .await
            .expect("cancel must wake without a signal, UDS, or EINTR")
            .expect("join");
        assert!(cancel.is_cancelled());
    }

    #[tokio::test]
    async fn portable_cancel_wakes_waiter_without_signal() {
        prove_portable_cancel_wakes_without_eintr_or_signal().await;
    }

    #[tokio::test]
    async fn portable_cancel_pretriggered_is_ready_without_clock() {
        let cancel = Cancel::new();
        cancel.trigger();
        tokio::time::timeout(Duration::from_secs(2), cancel.cancelled())
            .await
            .expect("pre-triggered cancel must not wait on a clock or EINTR");
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn portable_cancel_windows_uses_watch_token() {
        prove_portable_cancel_wakes_without_eintr_or_signal().await;
    }
}
