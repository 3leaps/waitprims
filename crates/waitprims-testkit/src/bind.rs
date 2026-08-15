//! Bind handles that release on drop.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use waitprims_async::BindHandle;
use waitprims_core::{Anchor, AnchorKind, IdToken, Registration};

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
    /// Exclusive start cursor resolved at bind. Never a policy label.
    pub resolved_start: Anchor,
    /// Held so [`BindInner::drop`] releases the tracker when the last clone goes away.
    #[allow(dead_code)]
    inner: Arc<BindInner>,
}

struct BindInner {
    registration_id: String,
    tracker: BindTracker,
}

impl TrackedBind {
    /// Track a new bind with the exclusive start cursor resolved at bind.
    pub fn new(registration_id: IdToken, resolved_start: Anchor, tracker: BindTracker) -> Self {
        let id = registration_id.as_str().to_string();
        Self {
            registration_id,
            resolved_start,
            inner: Arc::new(BindInner {
                registration_id: id,
                tracker,
            }),
        }
    }
}

impl BindHandle for TrackedBind {
    fn registration_id(&self) -> &IdToken {
        &self.registration_id
    }

    fn resolved_start(&self) -> &Anchor {
        &self.resolved_start
    }
}

/// Exclusive provider head assigned when a registration cites `baseline_policy`.
///
/// Derived from the registration id. Not a policy label such as
/// `anc:baseline-latest`.
pub fn exclusive_head_anchor(registration_id: &IdToken) -> Anchor {
    let rest = registration_id
        .as_str()
        .strip_prefix("reg:")
        .unwrap_or(registration_id.as_str());
    let local = if rest.len() > 58 { &rest[..58] } else { rest };
    Anchor {
        kind: AnchorKind::ProviderOpaque,
        value: IdToken::new(format!("anc:h-{local}")),
    }
}

/// Resolve the exclusive start cursor at bind.
///
/// An explicit `start_anchor` is kept. A `baseline_policy` becomes the
/// scripted head when one exists, otherwise [`exclusive_head_anchor`].
pub fn resolve_start_at_bind(registration: &Registration, script_head: Option<Anchor>) -> Anchor {
    if let Some(start) = &registration.start_anchor {
        return start.clone();
    }
    script_head.unwrap_or_else(|| exclusive_head_anchor(&registration.registration_id))
}

impl Drop for BindInner {
    fn drop(&mut self) {
        self.tracker.cancel(&self.registration_id);
    }
}
