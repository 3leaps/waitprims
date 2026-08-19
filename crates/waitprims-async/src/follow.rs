//! Held-follow runner: bind once, emit bursts, keep the session.
//!
//! Public JSON remains the six `agent-wait/v0` kinds.
//! [`FollowBurst`] and [`FollowEnd`] are runtime-only.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use waitprims_core::{
    AuthnMode, IdToken, LiveWaitRequest, NormativeReason, Registration, RegistrationSet, Result,
    Timestamp, ValidationError, WaitEvent,
};

use crate::cancel::Cancel;
use crate::clock::Clock;
use crate::observer::{Observation, Observer};
use crate::race::observation_is_replayable;

const BACKOFF_MS: &[u64] = &[50, 100, 200, 400, 800, 1000];

/// One readiness turn of accepted events, in registration-set order.
///
/// Not an `agent-wait/v0` message kind. Each event keeps
/// `proposed_next_anchor`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FollowBurst {
    /// Events dequeued in this turn, registration-set order.
    pub events: Vec<WaitEvent>,
}

/// Why a held-follow session stopped.
///
/// Not an `agent-wait/v0` message kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FollowEnd {
    /// Consumer cancelled before a later emit.
    Cancel,
    /// Applicable request deadline with required binds complete.
    Deadline,
    /// A fail-closed observer arm ended the session.
    TerminalArm {
        /// Registration that produced the terminal observation.
        registration_id: IdToken,
        /// Closed observer terminal kind.
        kind: TerminalArmKind,
        /// Stable reason code from the observer, or `buffer_overflow` /
        /// `required_bind_pending`.
        reason_code: IdToken,
    },
}

/// Observer terminals that end follow. Idle is not one of these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalArmKind {
    /// Bounded observer buffer overflowed.
    Overflow,
    /// Arm failed without a usable event.
    Failed,
    /// Provider outage on this arm.
    Outage,
    /// Exclusive cursor is uncertain.
    CursorUncertain,
    /// Arm is degraded.
    Degraded,
}

type BindFut<'a, O> = Pin<Box<dyn Future<Output = Result<<O as Observer>::Bind>> + Send + 'a>>;
type NextFut<'a> = Pin<Box<dyn Future<Output = Result<Observation>> + Send + 'a>>;

/// Bind once per registration and emit every accepted event until a
/// terminal. `on_burst` is backpressure: the runner does not call
/// [`Observer::next`] again until the sink returns `Ok`.
///
/// Lease rejects at `now >= lease_expires_at` (first-match stays `>`).
/// Authn/lease failures are [`ValidationError::normative`].
pub async fn run_follow<O, C, S, Fut>(
    observer: &O,
    clock: &C,
    cancel: &Cancel,
    set: &RegistrationSet,
    request: &LiveWaitRequest,
    mut on_burst: S,
) -> Result<FollowEnd>
where
    O: Observer,
    O::Bind: 'static,
    C: Clock,
    S: FnMut(FollowBurst) -> Fut,
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
    let mut backoff_step = 0usize;
    let outcome = loop {
        if cancel.is_cancelled() {
            break finish_cancel(observer, &mut slots);
        }
        if let Some(err) = posture_err(set, request, &clock.now()) {
            break finish_err(observer, &mut slots, err);
        }

        let wake = earliest_wake(set, request);
        let mut collect = CollectTurn { slots: &mut slots };
        tokio::select! {
            biased;
            result = &mut collect => {
                match result {
                    Err(err) => break finish_err(observer, &mut slots, err),
                    Ok(Turn::Idle) => {
                        slots.clear_nonreplayable();
                        backoff_step = backoff_step.saturating_add(1);
                        sleep_backoff(clock, cancel, set, request, backoff_step).await;
                        if let Some(done) = on_wake(
                            observer,
                            cancel,
                            set,
                            request,
                            &mut slots,
                            &clock.now(),
                            &mut on_burst,
                        )
                        .await
                        {
                            break done;
                        }
                    }
                    Ok(Turn::Ready) => {
                        if cancel.is_cancelled() {
                            break finish_cancel(observer, &mut slots);
                        }
                        if let Some(err) = posture_err(set, request, &clock.now()) {
                            break finish_err(observer, &mut slots, err);
                        }
                        let events = slots.take_events();
                        let terminal = slots.first_terminal(set);
                        if !events.is_empty() {
                            if let Err(err) = on_burst(FollowBurst { events }).await {
                                break Err(err);
                            }
                        }
                        if let Some(end) = terminal {
                            break Ok(end);
                        }
                        slots.rearm_idle();
                    }
                }
            }
            _ = cancel.cancelled() => {
                poll_once(&mut slots).await;
                break finish_cancel(observer, &mut slots);
            }
            _ = clock.sleep_until(&wake) => {
                if let Some(done) = on_wake(
                    observer,
                    cancel,
                    set,
                    request,
                    &mut slots,
                    &clock.now(),
                    &mut on_burst,
                )
                .await
                {
                    break done;
                }
            }
        }
    };
    drop(slots);
    outcome
}

fn resolve(set: &RegistrationSet, request: &LiveWaitRequest) -> Result<()> {
    if request.registration_set_ref.as_str() != set.message_id.as_str() {
        return Err(ValidationError::new("/registration_set_ref", "mismatch").into());
    }
    if request.registration_revision.as_str() != set.registration_revision.as_str() {
        return Err(ValidationError::new("/registration_revision", "revision_mismatch").into());
    }
    if request.waiter_id.as_str() != set.waiter_id.as_str() {
        return Err(ValidationError::new("/waiter_id", "mismatch").into());
    }
    Ok(())
}

fn posture_err(
    set: &RegistrationSet,
    request: &LiveWaitRequest,
    now: &Timestamp,
) -> Option<waitprims_core::Error> {
    if set
        .registrations
        .iter()
        .any(|reg| now >= &reg.lease_expires_at)
    {
        return Some(
            ValidationError::normative(
                "/registrations/lease_expires_at",
                "lease_expired",
                NormativeReason::LeaseReauth,
            )
            .into(),
        );
    }
    if set.authn_mode == AuthnMode::Required && request.verification_receipt_ref.is_none() {
        return Some(
            ValidationError::normative(
                "/verification_receipt_ref",
                "required",
                NormativeReason::AuthnRequired,
            )
            .into(),
        );
    }
    None
}

fn earliest_deadline(request: &LiveWaitRequest) -> &Timestamp {
    if request.run_deadline < request.logical_deadline {
        &request.run_deadline
    } else {
        &request.logical_deadline
    }
}

fn earliest_lease(set: &RegistrationSet) -> Option<&Timestamp> {
    set.registrations
        .iter()
        .map(|reg| &reg.lease_expires_at)
        .min()
}

fn earliest_wake(set: &RegistrationSet, request: &LiveWaitRequest) -> Timestamp {
    let deadline = earliest_deadline(request).clone();
    match earliest_lease(set) {
        Some(lease) if lease < &deadline => lease.clone(),
        _ => deadline,
    }
}

fn at_deadline(request: &LiveWaitRequest, now: &Timestamp) -> bool {
    now >= &request.logical_deadline || now >= &request.run_deadline
}

fn deadline_end<O: Observer>(
    set: &RegistrationSet,
    request: &LiveWaitRequest,
    now: &Timestamp,
    slots: &SlotSet<'_, O>,
) -> Option<FollowEnd>
where
    O::Bind: 'static,
{
    if !at_deadline(request, now) {
        return None;
    }
    if let Some(reg) = slots.pending_required(set) {
        return Some(FollowEnd::TerminalArm {
            registration_id: reg.registration_id.clone(),
            kind: TerminalArmKind::Failed,
            reason_code: IdToken::new("required_bind_pending"),
        });
    }
    if let Some(end) = slots.first_terminal(set) {
        return Some(end);
    }
    Some(FollowEnd::Deadline)
}

fn backoff_deadline<C: Clock>(
    clock: &C,
    set: &RegistrationSet,
    request: &LiveWaitRequest,
    step: usize,
) -> Timestamp {
    let idx = step.saturating_sub(1).min(BACKOFF_MS.len() - 1);
    let delay = Duration::from_millis(BACKOFF_MS[idx]);
    let wake = clock.now().saturating_add(delay);
    let cap = earliest_wake(set, request);
    if wake < cap {
        wake
    } else {
        cap
    }
}

async fn sleep_backoff<C: Clock>(
    clock: &C,
    cancel: &Cancel,
    set: &RegistrationSet,
    request: &LiveWaitRequest,
    step: usize,
) {
    let sleep_to = backoff_deadline(clock, set, request, step);
    tokio::select! {
        biased;
        _ = cancel.cancelled() => {}
        _ = clock.sleep_until(&sleep_to) => {}
    }
}

/// Harvest already-ready slots, then apply cancel / posture / deadline.
/// Used after request-deadline and lease wakes, and after Idle backoff
/// that may have been capped at those same boundaries.
async fn on_wake<O, S, Fut>(
    observer: &O,
    cancel: &Cancel,
    set: &RegistrationSet,
    request: &LiveWaitRequest,
    slots: &mut SlotSet<'_, O>,
    now: &Timestamp,
    on_burst: &mut S,
) -> Option<Result<FollowEnd>>
where
    O: Observer,
    O::Bind: 'static,
    S: FnMut(FollowBurst) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    poll_once(slots).await;
    slots.harvest_poll_ready(observer);
    if cancel.is_cancelled() {
        return Some(finish_cancel(observer, slots));
    }
    if let Some(err) = posture_err(set, request, now) {
        return Some(finish_err(observer, slots, err));
    }
    let events = slots.take_events();
    let terminal = slots.first_terminal(set);
    if !events.is_empty() {
        if let Err(err) = on_burst(FollowBurst { events }).await {
            return Some(Err(err));
        }
    }
    if let Some(end) = terminal {
        return Some(Ok(end));
    }
    if at_deadline(request, now) {
        if let Some(end) = deadline_end(set, request, now, slots) {
            return Some(Ok(end));
        }
    }
    slots.rearm_idle();
    None
}

async fn poll_once<O: Observer>(slots: &mut SlotSet<'_, O>)
where
    O::Bind: 'static,
{
    struct Once<'s, 'a, O: Observer> {
        slots: &'s mut SlotSet<'a, O>,
    }
    impl<O: Observer> Future for Once<'_, '_, O>
    where
        O::Bind: 'static,
    {
        type Output = ();
        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
            self.get_mut().slots.poll_available(cx);
            Poll::Ready(())
        }
    }
    Once { slots }.await;
}

fn finish_cancel<O: Observer>(observer: &O, slots: &mut SlotSet<'_, O>) -> Result<FollowEnd>
where
    O::Bind: 'static,
{
    restore_harvested(observer, slots)?;
    Ok(FollowEnd::Cancel)
}

fn finish_err<O: Observer>(
    observer: &O,
    slots: &mut SlotSet<'_, O>,
    err: waitprims_core::Error,
) -> Result<FollowEnd>
where
    O::Bind: 'static,
{
    restore_harvested(observer, slots)?;
    Err(err)
}

fn restore_harvested<O: Observer>(observer: &O, slots: &mut SlotSet<'_, O>) -> Result<()>
where
    O::Bind: 'static,
{
    let ready = slots.drain_replayable();
    let mut first_err = None;
    for (idx, obs) in ready {
        if let Some(bind) = slots.binds[idx].as_ref() {
            if let Err(err) = observer.restore_ready(bind.as_ref(), obs) {
                if first_err.is_none() {
                    first_err = Some(err);
                }
            }
        }
    }
    match first_err {
        Some(err) => Err(err),
        None => Ok(()),
    }
}

enum Turn {
    Idle,
    Ready,
}

struct SlotSet<'a, O: Observer> {
    observer: &'a O,
    registrations: &'a [Registration],
    binds: Vec<Option<Arc<O::Bind>>>,
    bind_futs: Vec<Option<BindFut<'a, O>>>,
    next_futs: Vec<Option<NextFut<'a>>>,
    harvested: Vec<Option<Observation>>,
    fault: Option<waitprims_core::Error>,
}

impl<'a, O: Observer> SlotSet<'a, O>
where
    O::Bind: 'static,
{
    fn new(observer: &'a O, registrations: &'a [Registration]) -> Self {
        let n = registrations.len();
        let mut bind_futs = Vec::with_capacity(n);
        for registration in registrations {
            let fut: BindFut<'a, O> = Box::pin(observer.bind(registration));
            bind_futs.push(Some(fut));
        }
        Self {
            observer,
            registrations,
            binds: (0..n).map(|_| None).collect(),
            bind_futs,
            next_futs: (0..n).map(|_| None).collect(),
            harvested: (0..n).map(|_| None).collect(),
            fault: None,
        }
    }

    fn len(&self) -> usize {
        self.registrations.len()
    }

    fn index_of(&self, registration_id: &str) -> Option<usize> {
        self.registrations
            .iter()
            .position(|reg| reg.registration_id.as_str() == registration_id)
    }

    fn arm_next(&mut self, idx: usize) {
        let observer = self.observer;
        let bind = Arc::clone(self.binds[idx].as_ref().expect("bound"));
        self.next_futs[idx] = Some(Box::pin(async move { observer.next(bind.as_ref()).await }));
    }

    fn poll_available(&mut self, cx: &mut Context<'_>) {
        loop {
            let mut progressed = false;
            for idx in 0..self.len() {
                if self.harvested[idx].is_some() {
                    continue;
                }
                if let Some(fut) = self.bind_futs[idx].as_mut() {
                    match fut.as_mut().poll(cx) {
                        Poll::Ready(Ok(bind)) => {
                            self.bind_futs[idx] = None;
                            self.binds[idx] = Some(Arc::new(bind));
                            self.arm_next(idx);
                            progressed = true;
                        }
                        Poll::Ready(Err(err)) => {
                            self.bind_futs[idx] = None;
                            if self.fault.is_none() {
                                self.fault = Some(err);
                            }
                        }
                        Poll::Pending => {}
                    }
                }
                if let Some(fut) = self.next_futs[idx].as_mut() {
                    match fut.as_mut().poll(cx) {
                        Poll::Ready(Ok(obs)) => {
                            self.next_futs[idx] = None;
                            self.harvested[idx] = Some(obs);
                            progressed = true;
                        }
                        Poll::Ready(Err(err)) => {
                            self.next_futs[idx] = None;
                            if self.fault.is_none() {
                                self.fault = Some(err);
                            }
                        }
                        Poll::Pending => {}
                    }
                }
            }
            if !progressed {
                break;
            }
        }
    }

    fn harvest_poll_ready(&mut self, observer: &O) {
        for idx in 0..self.len() {
            if self.harvested[idx].is_some() {
                continue;
            }
            let Some(bind) = self.binds[idx].as_ref() else {
                continue;
            };
            if let Some(obs) = observer.poll_ready(bind.as_ref()) {
                self.next_futs[idx] = None;
                self.harvested[idx] = Some(obs);
            }
        }
    }

    fn take_events(&mut self) -> Vec<WaitEvent> {
        let mut events = Vec::new();
        for slot in &mut self.harvested {
            if let Some(Observation::Event(_)) = slot {
                if let Some(Observation::Event(event)) = slot.take() {
                    events.push(*event);
                }
            }
        }
        events
    }

    fn first_terminal(&self, set: &RegistrationSet) -> Option<FollowEnd> {
        for (idx, obs) in self.harvested.iter().enumerate() {
            let Some(obs) = obs else { continue };
            let registration_id = set.registrations[idx].registration_id.clone();
            let end = match obs {
                Observation::Overflow => Some(FollowEnd::TerminalArm {
                    registration_id,
                    kind: TerminalArmKind::Overflow,
                    reason_code: IdToken::new("buffer_overflow"),
                }),
                Observation::Failed { reason_code } => Some(FollowEnd::TerminalArm {
                    registration_id,
                    kind: TerminalArmKind::Failed,
                    reason_code: reason_code.clone(),
                }),
                Observation::Outage { reason_code } => Some(FollowEnd::TerminalArm {
                    registration_id,
                    kind: TerminalArmKind::Outage,
                    reason_code: reason_code.clone(),
                }),
                Observation::CursorUncertain { reason_code } => Some(FollowEnd::TerminalArm {
                    registration_id,
                    kind: TerminalArmKind::CursorUncertain,
                    reason_code: reason_code.clone(),
                }),
                Observation::Degraded { reason_code } => Some(FollowEnd::TerminalArm {
                    registration_id,
                    kind: TerminalArmKind::Degraded,
                    reason_code: reason_code.clone(),
                }),
                Observation::Event(_) | Observation::Idle => None,
            };
            if end.is_some() {
                return end;
            }
        }
        None
    }

    fn pending_required<'s>(&'s self, set: &'s RegistrationSet) -> Option<&'s Registration> {
        set.registrations.iter().find(|reg| {
            if !reg.required {
                return false;
            }
            let Some(idx) = self.index_of(reg.registration_id.as_str()) else {
                return true;
            };
            self.binds[idx].is_none()
        })
    }

    fn drain_replayable(&mut self) -> Vec<(usize, Observation)> {
        let mut out = Vec::new();
        for (idx, slot) in self.harvested.iter_mut().enumerate() {
            if let Some(obs) = slot.take() {
                if observation_is_replayable(&obs) {
                    out.push((idx, obs));
                }
            }
        }
        out
    }

    fn clear_nonreplayable(&mut self) {
        for slot in &mut self.harvested {
            if slot
                .as_ref()
                .is_some_and(|obs| !observation_is_replayable(obs))
            {
                *slot = None;
            }
        }
    }

    fn rearm_idle(&mut self) {
        self.clear_nonreplayable();
        for idx in 0..self.len() {
            if self.binds[idx].is_some()
                && self.next_futs[idx].is_none()
                && self.harvested[idx].is_none()
            {
                self.arm_next(idx);
            }
        }
    }

    fn all_idle_bound(&self) -> bool {
        if self.registrations.is_empty() {
            return false;
        }
        self.binds.iter().all(|bind| bind.is_some())
            && self
                .harvested
                .iter()
                .all(|obs| matches!(obs, Some(Observation::Idle)))
    }

    fn has_terminal_obs(&self) -> bool {
        self.harvested.iter().any(|obs| {
            obs.as_ref()
                .is_some_and(|obs| !matches!(obs, Observation::Idle))
        })
    }
}

struct CollectTurn<'s, 'a, O: Observer> {
    slots: &'s mut SlotSet<'a, O>,
}

impl<O: Observer> Future for CollectTurn<'_, '_, O>
where
    O::Bind: 'static,
{
    type Output = Result<Turn>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let slots = &mut self.get_mut().slots;
        slots.poll_available(cx);
        if let Some(err) = slots.fault.take() {
            return Poll::Ready(Err(err));
        }
        if slots.has_terminal_obs() {
            return Poll::Ready(Ok(Turn::Ready));
        }
        if slots.all_idle_bound() {
            return Poll::Ready(Ok(Turn::Idle));
        }
        Poll::Pending
    }
}
