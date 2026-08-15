//! Bind handles that release on drop.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use waitprims_core::IdToken;

/// Shared live/cancelled registration set.
#[derive(Clone, Default)]
pub struct BindTracker {
    inner: Arc<Mutex<TrackerState>>,
}

#[derive(Default)]
struct TrackerState {
    live: BTreeSet<String>,
    cancelled: BTreeSet<String>,
}

impl BindTracker {
    /// Empty tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a live bind.
    pub fn acquire(&self, registration_id: &str) {
        let mut state = self.inner.lock().expect("tracker");
        state.live.insert(registration_id.to_string());
    }

    /// Mark a bind cancelled and no longer live.
    pub fn cancel(&self, registration_id: &str) {
        let mut state = self.inner.lock().expect("tracker");
        state.live.remove(registration_id);
        state.cancelled.insert(registration_id.to_string());
    }

    /// Drop a live bind without recording cancel.
    pub fn release(&self, registration_id: &str) {
        self.inner
            .lock()
            .expect("tracker")
            .live
            .remove(registration_id);
    }

    /// Number of binds that have not been released.
    pub fn live_count(&self) -> usize {
        self.inner.lock().expect("tracker").live.len()
    }

    /// Registration ids that were explicitly cancelled.
    pub fn cancelled_ids(&self) -> BTreeSet<String> {
        self.inner.lock().expect("tracker").cancelled.clone()
    }
}

/// Bind handle. The last clone releases the registration.
pub struct TrackedBind {
    /// Registration this bind observes.
    pub registration_id: IdToken,
    /// Held so [`BindInner::drop`] releases the tracker when the last clone goes away.
    #[allow(dead_code)]
    inner: Arc<BindInner>,
}

struct BindInner {
    registration_id: String,
    tracker: BindTracker,
}

impl TrackedBind {
    /// Track a new bind.
    pub fn new(registration_id: IdToken, tracker: BindTracker) -> Self {
        let id = registration_id.as_str().to_string();
        Self {
            registration_id,
            inner: Arc::new(BindInner {
                registration_id: id,
                tracker,
            }),
        }
    }
}

impl Drop for BindInner {
    fn drop(&mut self) {
        self.tracker.cancel(&self.registration_id);
    }
}
