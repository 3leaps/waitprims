//! Scripted and idle observers.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use waitprims_async::{Observation, Observer};
use waitprims_core::{Anchor, IdToken, Registration, Result, Timestamp, WaitEvent};

use crate::bind::{resolve_start_at_bind, BindTracker, TrackedBind};
use crate::case::wait_event;
use crate::clock::FakeClock;
use crate::script::Script;

#[derive(Clone)]
enum ArmFault {
    Outage(IdToken),
    CursorUncertain(IdToken),
    Degraded(IdToken),
    Failed(IdToken),
}

struct Queues {
    buffer_limit: usize,
    events: Mutex<BTreeMap<String, VecDeque<WaitEvent>>>,
    overflowed: Mutex<BTreeSet<String>>,
    faults: Mutex<BTreeMap<String, ArmFault>>,
}

/// Observer that emits scripted events at their `observed_at` times.
#[derive(Clone)]
pub struct ScriptedObserver {
    queues: Arc<Queues>,
    clock: FakeClock,
    tracker: BindTracker,
    hang_binds: Arc<Mutex<BTreeSet<String>>>,
    hang_cancel: Arc<AtomicBool>,
    bind_requested: Arc<Mutex<BTreeMap<String, Option<Anchor>>>>,
    bind_resolved: Arc<Mutex<BTreeMap<String, Anchor>>>,
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
                faults: Mutex::new(BTreeMap::new()),
            }),
            clock,
            tracker: BindTracker::new(),
            hang_binds: Arc::new(Mutex::new(BTreeSet::new())),
            hang_cancel: Arc::new(AtomicBool::new(false)),
            bind_requested: Arc::new(Mutex::new(BTreeMap::new())),
            bind_resolved: Arc::new(Mutex::new(BTreeMap::new())),
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

    /// Report provider outage on this registration.
    pub fn outage(&self, registration_id: &str, reason_code: &str) {
        self.set_fault(registration_id, ArmFault::Outage(IdToken::new(reason_code)));
    }

    /// Report an uncertain exclusive cursor on this registration.
    pub fn cursor_uncertain(&self, registration_id: &str, reason_code: &str) {
        self.set_fault(
            registration_id,
            ArmFault::CursorUncertain(IdToken::new(reason_code)),
        );
    }

    /// Report a degraded required arm.
    pub fn degrade(&self, registration_id: &str, reason_code: &str) {
        self.set_fault(
            registration_id,
            ArmFault::Degraded(IdToken::new(reason_code)),
        );
    }

    /// Report a failed arm without a usable event.
    pub fn fail_arm(&self, registration_id: &str, reason_code: &str) {
        self.set_fault(registration_id, ArmFault::Failed(IdToken::new(reason_code)));
    }

    fn set_fault(&self, registration_id: &str, fault: ArmFault) {
        self.queues
            .faults
            .lock()
            .expect("observer")
            .insert(registration_id.to_string(), fault);
    }

    fn fault_observation(&self, registration_id: &str) -> Option<Observation> {
        self.queues
            .faults
            .lock()
            .expect("observer")
            .get(registration_id)
            .map(|fault| match fault {
                ArmFault::Outage(reason) => Observation::Outage {
                    reason_code: reason.clone(),
                },
                ArmFault::CursorUncertain(reason) => Observation::CursorUncertain {
                    reason_code: reason.clone(),
                },
                ArmFault::Degraded(reason) => Observation::Degraded {
                    reason_code: reason.clone(),
                },
                ArmFault::Failed(reason) => Observation::Failed {
                    reason_code: reason.clone(),
                },
            })
    }

    /// Binds that have not been released.
    pub fn live_bind_count(&self) -> usize {
        self.tracker.live_count()
    }

    /// Registration ids passed to [`Observer::cancel`].
    pub fn cancelled_ids(&self) -> BTreeSet<String> {
        self.tracker.cancelled_ids()
    }

    /// `start_anchor` on the registration passed to [`Observer::bind`].
    pub fn bind_requested_starts(&self) -> BTreeMap<String, Option<Anchor>> {
        self.bind_requested.lock().expect("observer").clone()
    }

    /// Exclusive cursor resolved at bind.
    pub fn bind_resolved_starts(&self) -> BTreeMap<String, Anchor> {
        self.bind_resolved.lock().expect("observer").clone()
    }

    /// Event ids still queued, front first, keyed by registration id.
    pub fn queued_event_ids(&self) -> BTreeMap<String, Vec<String>> {
        self.queues
            .events
            .lock()
            .expect("observer")
            .iter()
            .map(|(rid, queue)| {
                (
                    rid.clone(),
                    queue
                        .iter()
                        .map(|event| event.event_id.as_str().to_string())
                        .collect(),
                )
            })
            .collect()
    }

    fn take_due(&self, registration_id: &str) -> Option<Observation> {
        if let Some(fault) = self.fault_observation(registration_id) {
            return Some(fault);
        }
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
        let resolved = resolve_start_at_bind(registration, script_head);
        self.bind_requested.lock().expect("observer").insert(
            registration.registration_id.as_str().to_string(),
            registration.start_anchor.clone(),
        );
        self.bind_resolved.lock().expect("observer").insert(
            registration.registration_id.as_str().to_string(),
            resolved.clone(),
        );
        Ok(TrackedBind::new(
            registration.registration_id.clone(),
            resolved,
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

    fn restore_ready(&self, bind: &Self::Bind, obs: Observation) {
        if let Observation::Event(event) = obs {
            self.queues
                .events
                .lock()
                .expect("observer")
                .entry(bind.registration_id.as_str().to_string())
                .or_default()
                .push_front(*event);
        }
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

    fn restore_ready(&self, _bind: &Self::Bind, _obs: Observation) {
        // `poll_ready` is the default no-dequeue impl; `next` synthesizes Idle.
    }
}

/// Observer whose [`Observer::poll_ready`] never runs dry.
///
/// Used to prove collection stops at representable bounds instead of
/// buffering an unbounded ready stream.
#[derive(Clone, Default)]
pub struct EndlessReadyObserver {
    tracker: BindTracker,
    seq: Arc<AtomicU64>,
    restored: Arc<Mutex<BTreeMap<String, VecDeque<WaitEvent>>>>,
}

impl EndlessReadyObserver {
    /// Continuously-ready observer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Binds that have not been released.
    pub fn live_bind_count(&self) -> usize {
        self.tracker.live_count()
    }

    fn mint(&self, bind: &TrackedBind) -> WaitEvent {
        let n = self.seq.fetch_add(1, Ordering::Relaxed);
        let mut event = wait_event(
            bind.registration_id.as_str(),
            "method",
            &format!("evt:endless-{n}"),
            "2026-08-15T16:01:00Z",
        );
        event.start_anchor = bind.resolved_start.clone();
        event.proposed_next_anchor = Anchor {
            kind: event.proposed_next_anchor.kind,
            value: IdToken::new(format!("anc:after-endless-{n}")),
        };
        event
    }

    fn take(&self, bind: &TrackedBind) -> WaitEvent {
        if let Some(event) = self
            .restored
            .lock()
            .expect("observer")
            .get_mut(bind.registration_id.as_str())
            .and_then(|queue| queue.pop_front())
        {
            return event;
        }
        self.mint(bind)
    }
}

impl Observer for EndlessReadyObserver {
    type Bind = TrackedBind;

    async fn bind(&self, registration: &Registration) -> Result<Self::Bind> {
        self.tracker.acquire(registration.registration_id.as_str());
        Ok(TrackedBind::new(
            registration.registration_id.clone(),
            resolve_start_at_bind(registration, None),
            self.tracker.clone(),
        ))
    }

    async fn next(&self, bind: &Self::Bind) -> Result<Observation> {
        Ok(Observation::Event(Box::new(self.take(bind))))
    }

    async fn cancel(&self, bind: &Self::Bind) -> Result<()> {
        self.tracker.cancel(bind.registration_id.as_str());
        Ok(())
    }

    fn poll_ready(&self, bind: &Self::Bind) -> Option<Observation> {
        Some(Observation::Event(Box::new(self.take(bind))))
    }

    fn restore_ready(&self, bind: &Self::Bind, obs: Observation) {
        if let Observation::Event(event) = obs {
            self.restored
                .lock()
                .expect("observer")
                .entry(bind.registration_id.as_str().to_string())
                .or_default()
                .push_front(*event);
        }
    }
}
