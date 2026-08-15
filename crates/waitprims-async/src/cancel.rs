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
