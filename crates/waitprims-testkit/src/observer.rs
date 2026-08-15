//! Scripted and idle observers.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use waitprims_async::{Observation, Observer};
use waitprims_core::{Anchor, Registration, Result, Timestamp, WaitEvent};

use crate::bind::{resolve_start_at_bind, BindTracker, TrackedBind};
use crate::clock::FakeClock;
use crate::script::Script;

struct Queues {
    buffer_limit: usize,
    events: Mutex<BTreeMap<String, VecDeque<WaitEvent>>>,
    overflowed: Mutex<BTreeSet<String>>,
}

/// Observer that emits scripted events at their `observed_at` times.
#[derive(Clone)]
pub struct ScriptedObserver {
    queues: Arc<Queues>,
    clock: FakeClock,
    tracker: BindTracker,
    hang_binds: Arc<Mutex<BTreeSet<String>>>,
    hang_cancel: Arc<AtomicBool>,
}

impl ScriptedObserver {
    /// Build from a local script and clock.
    pub fn new(script: Script, clock: FakeClock) -> Self {
        let mut events: BTreeMap<String, Vec<WaitEvent>> = BTreeMap::new();
        for event in script.events {
            events
                .entry(event.registration_id.as_str().to_string())
                .or_default()
                .push(event);
        }
        let mut queued = BTreeMap::new();
        for (id, mut list) in events {
            list.sort_by(|left, right| {
                left.observed_at
                    .cmp(&right.observed_at)
                    .then_with(|| left.event_id.as_str().cmp(right.event_id.as_str()))
            });
            queued.insert(id, VecDeque::from(list));
        }
        Self {
            queues: Arc::new(Queues {
                buffer_limit: script.buffer_limit.max(1),
                events: Mutex::new(queued),
                overflowed: Mutex::new(BTreeSet::new()),
            }),
            clock,
            tracker: BindTracker::new(),
            hang_binds: Arc::new(Mutex::new(BTreeSet::new())),
            hang_cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Leave `bind()` pending forever for this registration.
    pub fn hang_bind(&self, registration_id: &str) {
        self.hang_binds
            .lock()
            .expect("observer")
            .insert(registration_id.to_string());
    }

    /// Leave `cancel()` pending forever. Drop remains the release guarantee.
    pub fn hang_cancel(&self) {
        self.hang_cancel.store(true, Ordering::Relaxed);
    }

    /// Binds that have not been released.
    pub fn live_bind_count(&self) -> usize {
        self.tracker.live_count()
    }

    /// Registration ids passed to [`Observer::cancel`].
    pub fn cancelled_ids(&self) -> BTreeSet<String> {
        self.tracker.cancelled_ids()
    }

    fn take_due(&self, registration_id: &str) -> Option<Observation> {
        let now = self.clock.current_time();
        if self
            .queues
            .overflowed
            .lock()
            .expect("observer")
            .contains(registration_id)
        {
            return Some(Observation::Overflow);
        }
        let mut events = self.queues.events.lock().expect("observer");
        let queue = events.get_mut(registration_id)?;
        let due = queue
            .iter()
            .filter(|event| event.observed_at <= now)
            .count();
        if due > self.queues.buffer_limit {
            self.queues
                .overflowed
                .lock()
                .expect("observer")
                .insert(registration_id.to_string());
            return Some(Observation::Overflow);
        }
        if due == 0 {
            return None;
        }
        queue
            .pop_front()
            .map(|event| Observation::Event(Box::new(event)))
    }

    fn next_ready_at(&self, registration_id: &str) -> Option<Timestamp> {
        self.queues
            .events
            .lock()
            .expect("observer")
            .get(registration_id)
            .and_then(|queue| queue.front().map(|event| event.observed_at.clone()))
    }

    fn script_head(&self, registration_id: &str) -> Option<Anchor> {
        self.queues
            .events
            .lock()
            .expect("observer")
            .get(registration_id)
            .and_then(|queue| queue.front().map(|event| event.start_anchor.clone()))
    }
}

impl Observer for ScriptedObserver {
    type Bind = TrackedBind;

    async fn bind(&self, registration: &Registration) -> Result<Self::Bind> {
        if self
            .hang_binds
            .lock()
            .expect("observer")
            .contains(registration.registration_id.as_str())
        {
            std::future::pending::<()>().await;
        }
        self.tracker.acquire(registration.registration_id.as_str());
        let script_head = self.script_head(registration.registration_id.as_str());
        Ok(TrackedBind::new(
            registration.registration_id.clone(),
            resolve_start_at_bind(registration, script_head),
            self.tracker.clone(),
        ))
    }

    async fn next(&self, bind: &Self::Bind) -> Result<Observation> {
        let id = bind.registration_id.as_str();
        loop {
            if let Some(obs) = self.take_due(id) {
                return Ok(obs);
            }
            match self.next_ready_at(id) {
                Some(when) => self.clock.sleep_to(&when).await,
                None => std::future::pending::<()>().await,
            }
        }
    }

    async fn cancel(&self, bind: &Self::Bind) -> Result<()> {
        if self.hang_cancel.load(Ordering::Relaxed) {
            std::future::pending::<()>().await;
        }
        self.tracker.cancel(bind.registration_id.as_str());
        Ok(())
    }

    fn poll_ready(&self, bind: &Self::Bind) -> Option<Observation> {
        self.take_due(bind.registration_id.as_str())
    }
}

/// Observer that always returns [`Observation::Idle`].
#[derive(Clone, Default)]
pub struct IdleObserver {
    tracker: BindTracker,
}

impl IdleObserver {
    /// Empty idle observer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Binds that have not been released.
    pub fn live_bind_count(&self) -> usize {
        self.tracker.live_count()
    }

    /// Registration ids passed to [`Observer::cancel`].
    pub fn cancelled_ids(&self) -> BTreeSet<String> {
        self.tracker.cancelled_ids()
    }
}

impl Observer for IdleObserver {
    type Bind = TrackedBind;

    async fn bind(&self, registration: &Registration) -> Result<Self::Bind> {
        self.tracker.acquire(registration.registration_id.as_str());
        Ok(TrackedBind::new(
            registration.registration_id.clone(),
            resolve_start_at_bind(registration, None),
            self.tracker.clone(),
        ))
    }

    async fn next(&self, _bind: &Self::Bind) -> Result<Observation> {
        Ok(Observation::Idle)
    }

    async fn cancel(&self, bind: &Self::Bind) -> Result<()> {
        self.tracker.cancel(bind.registration_id.as_str());
        Ok(())
    }
}
