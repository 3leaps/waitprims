//! Held-session emit policy: bind once, coalesce quiet, flush urgent.
//!
//! [`CoalesceBurst`] and [`CoalescePolicy`] are runtime-only. Public JSON
//! remains the six `agent-wait/v0` kinds. `priority` is a presentation
//! hint, not authorization.

use std::collections::BTreeMap;
use std::future::Future;
use std::time::Duration;

use waitprims_core::{
    IdToken, LiveWaitRequest, NormativeReason, Registration, RegistrationSet, Result, Timestamp,
    ValidationError, WaitEvent, PRIORITY_NORMAL, PRIORITY_URGENT,
};

use crate::cancel::Cancel;
use crate::clock::Clock;
use crate::follow::{
    at_deadline, deadline_end, earliest_wake, finish_err, poll_once, posture_err, resolve,
    sleep_backoff, CollectTurn, FollowEnd, SlotSet, TerminalArmKind, Turn,
};
use crate::observer::Observer;
use crate::poll_cycle::event_surface_bytes;

/// One coalesced emit spanning one or more readiness turns.
///
/// Not an `agent-wait/v0` message kind. Not [`crate::FollowBurst`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoalesceBurst {
    /// Events in turn FIFO, then registration-set order, per-registration FIFO.
    pub events: Vec<WaitEvent>,
}

/// Session emit policy. Not a wire message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoalescePolicy {
    /// Minimum gap between non-terminal quiet emits.
    pub min_emit_interval: Duration,
    /// Effective priority at or above this value flushes immediately.
    pub urgent_at: u8,
}

impl CoalescePolicy {
    /// Quiet window `min_emit_interval` with `urgent_at` = 100.
    pub fn new(min_emit_interval: Duration) -> Self {
        Self {
            min_emit_interval,
            urgent_at: PRIORITY_URGENT,
        }
    }
}

struct Pending {
    events: Vec<WaitEvent>,
    total_events: u64,
    total_bytes: u64,
    per_events: BTreeMap<String, u64>,
    per_bytes: BTreeMap<String, u64>,
}

impl Pending {
    fn new() -> Self {
        Self {
            events: Vec::new(),
            total_events: 0,
            total_bytes: 0,
            per_events: BTreeMap::new(),
            per_bytes: BTreeMap::new(),
        }
    }

    fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    fn take(&mut self) -> Vec<WaitEvent> {
        self.total_events = 0;
        self.total_bytes = 0;
        self.per_events.clear();
        self.per_bytes.clear();
        std::mem::take(&mut self.events)
    }

    fn try_push(
        &mut self,
        set: &RegistrationSet,
        event: WaitEvent,
    ) -> std::result::Result<(), IdToken> {
        let rid = event.registration_id.as_str().to_string();
        let bytes = event_surface_bytes(&event);
        let next_events = self.total_events.saturating_add(1);
        let next_bytes = self.total_bytes.saturating_add(bytes);
        if next_events > set.aggregate_limits.max_events
            || next_bytes > set.aggregate_limits.max_bytes
        {
            return Err(event.registration_id);
        }
        let Some(reg) = registration(set, &event.registration_id) else {
            return Err(event.registration_id);
        };
        let per_e = self
            .per_events
            .get(&rid)
            .copied()
            .unwrap_or(0)
            .saturating_add(1);
        let per_b = self
            .per_bytes
            .get(&rid)
            .copied()
            .unwrap_or(0)
            .saturating_add(bytes);
        if per_e > reg.bounds.max_events || per_b > reg.bounds.max_bytes {
            return Err(event.registration_id);
        }
        self.total_events = next_events;
        self.total_bytes = next_bytes;
        self.per_events.insert(rid.clone(), per_e);
        self.per_bytes.insert(rid, per_b);
        self.events.push(event);
        Ok(())
    }
}

fn registration<'a>(set: &'a RegistrationSet, id: &IdToken) -> Option<&'a Registration> {
    set.registrations
        .iter()
        .find(|reg| reg.registration_id.as_str() == id.as_str())
}

fn effective_priority(reg: &Registration) -> u8 {
    reg.priority.unwrap_or(PRIORITY_NORMAL)
}

fn is_urgent(set: &RegistrationSet, event: &WaitEvent, urgent_at: u8) -> bool {
    registration(set, &event.registration_id)
        .map(|reg| effective_priority(reg) >= urgent_at)
        .unwrap_or(false)
}

fn fail_closed<O: Observer>(
    observer: &O,
    slots: &mut SlotSet<'_, O>,
    pending: &mut Pending,
    err: waitprims_core::Error,
) -> Result<FollowEnd>
where
    O::Bind: 'static,
{
    let buffered = pending.take();
    slots.restore_events(observer, buffered)?;
    finish_err(observer, slots, err)
}

fn overflow_end(registration_id: IdToken) -> FollowEnd {
    FollowEnd::TerminalArm {
        registration_id,
        kind: TerminalArmKind::Overflow,
        reason_code: IdToken::new("buffer_overflow"),
    }
}

/// Bind once per registration and emit coalesced bursts until a terminal.
///
/// `on_burst` is backpressure: the runner does not call [`Observer::next`]
/// again until the sink returns `Ok`. Sink `Err` drops the pending buffer
/// and releases binds with no replay. Drop of the future releases binds
/// and does not flush.
pub async fn run_coalesce<O, C, S, Fut>(
    observer: &O,
    clock: &C,
    cancel: &Cancel,
    set: &RegistrationSet,
    request: &LiveWaitRequest,
    policy: &CoalescePolicy,
    mut on_burst: S,
) -> Result<FollowEnd>
where
    O: Observer,
    O::Bind: 'static,
    C: Clock,
    S: FnMut(CoalesceBurst) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    resolve(set, request)?;
    if request.run_deadline > request.logical_deadline {
        return Err(ValidationError::normative(
            "/run_deadline",
            "must_be_at_or_before_logical_deadline",
            NormativeReason::DeadlineOrdering,
        )
        .into());
    }
    if let Some(err) = posture_err(set, request, &clock.now()) {
        return Err(err);
    }

    let mut slots = SlotSet::new(observer, &set.registrations);
    let mut pending = Pending::new();
    let mut next_quiet_emit_at: Option<Timestamp> = None;
    let mut backoff_step = 0usize;
    let outcome = loop {
        if let Some(err) = posture_err(set, request, &clock.now()) {
            break fail_closed(observer, &mut slots, &mut pending, err);
        }
        let now = clock.now();
        if cancel.is_cancelled()
            || at_deadline(request, &now)
            || (!pending.is_empty() && next_quiet_emit_at.as_ref().is_some_and(|at| now >= *at))
        {
            poll_once(&mut slots).await;
            slots.harvest_poll_ready(observer);
            if let Some(done) = decide_coalesce(
                observer,
                cancel,
                set,
                request,
                policy,
                &mut slots,
                &now,
                &mut pending,
                &mut next_quiet_emit_at,
                &mut on_burst,
                cancel.is_cancelled(),
            )
            .await
            {
                break done;
            }
        }
        let wake = earliest_wake(set, request);
        let quiet_at = if pending.is_empty() {
            None
        } else {
            next_quiet_emit_at.clone()
        };
        let mut collect = CollectTurn { slots: &mut slots };
        tokio::select! {
            biased;
            result = &mut collect => {
                match result {
                    Err(err) => {
                        if let Some(posture) = posture_err(set, request, &clock.now()) {
                            break fail_closed(observer, &mut slots, &mut pending, posture);
                        }
                        break fail_closed(observer, &mut slots, &mut pending, err);
                    }
                    Ok(Turn::Idle) => {
                        backoff_step = backoff_step.saturating_add(1);
                        sleep_backoff(clock, cancel, set, request, backoff_step).await;
                        slots.rearm_idle();
                        poll_once(&mut slots).await;
                        slots.harvest_poll_ready(observer);
                        if let Some(done) = decide_coalesce(
                            observer,
                            cancel,
                            set,
                            request,
                            policy,
                            &mut slots,
                            &clock.now(),
                            &mut pending,
                            &mut next_quiet_emit_at,
                            &mut on_burst,
                            false,
                        )
                        .await
                        {
                            break done;
                        }
                    }
                    Ok(Turn::Ready) => {
                        if let Some(done) = decide_coalesce(
                            observer,
                            cancel,
                            set,
                            request,
                            policy,
                            &mut slots,
                            &clock.now(),
                            &mut pending,
                            &mut next_quiet_emit_at,
                            &mut on_burst,
                            false,
                        )
                        .await
                        {
                            break done;
                        }
                    }
                }
            }
            _ = cancel.cancelled() => {
                poll_once(&mut slots).await;
                slots.harvest_poll_ready(observer);
                if let Some(done) = decide_coalesce(
                    observer,
                    cancel,
                    set,
                    request,
                    policy,
                    &mut slots,
                    &clock.now(),
                    &mut pending,
                    &mut next_quiet_emit_at,
                    &mut on_burst,
                    true,
                )
                .await
                {
                    break done;
                }
            }
            _ = clock.sleep_until(&wake) => {
                slots.rearm_idle();
                poll_once(&mut slots).await;
                slots.harvest_poll_ready(observer);
                if let Some(done) = decide_coalesce(
                    observer,
                    cancel,
                    set,
                    request,
                    policy,
                    &mut slots,
                    &clock.now(),
                    &mut pending,
                    &mut next_quiet_emit_at,
                    &mut on_burst,
                    false,
                )
                .await
                {
                    break done;
                }
            }
            _ = async {
                if let Some(at) = quiet_at.as_ref() {
                    clock.sleep_until(at).await;
                } else {
                    std::future::pending::<()>().await;
                }
            } => {
                slots.rearm_idle();
                poll_once(&mut slots).await;
                slots.harvest_poll_ready(observer);
                if let Some(done) = decide_coalesce(
                    observer,
                    cancel,
                    set,
                    request,
                    policy,
                    &mut slots,
                    &clock.now(),
                    &mut pending,
                    &mut next_quiet_emit_at,
                    &mut on_burst,
                    false,
                )
                .await
                {
                    break done;
                }
            }
        }
    };
    drop(slots);
    drop(pending);
    outcome
}

#[allow(clippy::too_many_arguments)]
async fn decide_coalesce<O, S, Fut>(
    observer: &O,
    cancel: &Cancel,
    set: &RegistrationSet,
    request: &LiveWaitRequest,
    policy: &CoalescePolicy,
    slots: &mut SlotSet<'_, O>,
    now: &Timestamp,
    pending: &mut Pending,
    next_quiet_emit_at: &mut Option<Timestamp>,
    on_burst: &mut S,
    cancel_arm: bool,
) -> Option<Result<FollowEnd>>
where
    O: Observer,
    O::Bind: 'static,
    S: FnMut(CoalesceBurst) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    if let Some(err) = slots.fault.take() {
        return Some(fail_closed(observer, slots, pending, err));
    }
    let events = slots.take_events();
    let terminal = slots.first_terminal(set);
    let cancelled = cancel_arm || cancel.is_cancelled();
    let deadline = at_deadline(request, now);

    if terminal.is_some() || cancelled || deadline {
        return Some(
            final_flush(
                set, request, slots, now, pending, events, terminal, cancelled, on_burst,
            )
            .await,
        );
    }

    if let Some(done) = apply_turn(
        set,
        policy,
        now,
        pending,
        next_quiet_emit_at,
        events,
        on_burst,
    )
    .await
    {
        return Some(done);
    }
    slots.rearm_idle();
    None
}

#[allow(clippy::too_many_arguments)]
async fn final_flush<O, S, Fut>(
    set: &RegistrationSet,
    request: &LiveWaitRequest,
    slots: &SlotSet<'_, O>,
    now: &Timestamp,
    pending: &mut Pending,
    events: Vec<WaitEvent>,
    terminal: Option<FollowEnd>,
    cancelled: bool,
    on_burst: &mut S,
) -> Result<FollowEnd>
where
    O: Observer,
    O::Bind: 'static,
    S: FnMut(CoalesceBurst) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    for event in events {
        if let Err(rid) = pending.try_push(set, event) {
            pending.take();
            return Ok(overflow_end(rid));
        }
    }
    if !pending.is_empty() {
        let events = pending.take();
        on_burst(CoalesceBurst { events }).await?;
    }
    if let Some(end) = terminal {
        return Ok(end);
    }
    if cancelled {
        return Ok(FollowEnd::Cancel);
    }
    if let Some(end) = deadline_end(set, request, now, slots) {
        return Ok(end);
    }
    Ok(FollowEnd::Deadline)
}

#[allow(clippy::too_many_arguments)]
async fn apply_turn<S, Fut>(
    set: &RegistrationSet,
    policy: &CoalescePolicy,
    now: &Timestamp,
    pending: &mut Pending,
    next_quiet_emit_at: &mut Option<Timestamp>,
    events: Vec<WaitEvent>,
    on_burst: &mut S,
) -> Option<Result<FollowEnd>>
where
    S: FnMut(CoalesceBurst) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    if events.is_empty() {
        if interval_is_due(next_quiet_emit_at, now) && !pending.is_empty() {
            return emit_quiet(now, policy, pending, next_quiet_emit_at, on_burst).await;
        }
        return None;
    }

    if next_quiet_emit_at.is_none()
        && events
            .iter()
            .any(|event| !is_urgent(set, event, policy.urgent_at))
    {
        *next_quiet_emit_at = Some(now.saturating_add(policy.min_emit_interval));
    }
    let interval_due = interval_is_due(next_quiet_emit_at, now);
    let any_urgent = events
        .iter()
        .any(|event| is_urgent(set, event, policy.urgent_at));

    if any_urgent && interval_due {
        for event in events {
            if let Err(rid) = pending.try_push(set, event) {
                pending.take();
                return Some(Ok(overflow_end(rid)));
            }
        }
        return emit_quiet(now, policy, pending, next_quiet_emit_at, on_burst).await;
    }

    if any_urgent {
        let mut urgent = Vec::new();
        for event in events {
            if is_urgent(set, &event, policy.urgent_at) {
                urgent.push(event);
            } else if let Err(rid) = pending.try_push(set, event) {
                pending.take();
                return Some(Ok(overflow_end(rid)));
            } else if next_quiet_emit_at.is_none() {
                *next_quiet_emit_at = Some(now.saturating_add(policy.min_emit_interval));
            }
        }
        if !urgent.is_empty() {
            if let Err(err) = on_burst(CoalesceBurst { events: urgent }).await {
                pending.take();
                return Some(Err(err));
            }
        }
        return None;
    }

    for event in events {
        if let Err(rid) = pending.try_push(set, event) {
            pending.take();
            return Some(Ok(overflow_end(rid)));
        }
    }
    if next_quiet_emit_at.is_none() {
        *next_quiet_emit_at = Some(now.saturating_add(policy.min_emit_interval));
    }
    if interval_is_due(next_quiet_emit_at, now) {
        return emit_quiet(now, policy, pending, next_quiet_emit_at, on_burst).await;
    }
    None
}

fn interval_is_due(next_quiet_emit_at: &Option<Timestamp>, now: &Timestamp) -> bool {
    next_quiet_emit_at.as_ref().is_some_and(|at| now >= at)
}

async fn emit_quiet<S, Fut>(
    now: &Timestamp,
    policy: &CoalescePolicy,
    pending: &mut Pending,
    next_quiet_emit_at: &mut Option<Timestamp>,
    on_burst: &mut S,
) -> Option<Result<FollowEnd>>
where
    S: FnMut(CoalesceBurst) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    if pending.is_empty() {
        return None;
    }
    let events = pending.take();
    if let Err(err) = on_burst(CoalesceBurst { events }).await {
        return Some(Err(err));
    }
    *next_quiet_emit_at = Some(now.saturating_add(policy.min_emit_interval));
    None
}
