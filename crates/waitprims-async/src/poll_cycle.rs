//! Bounded poll-cycle runner.
//!
//! One cycle: bind and observe in one race, allocate visit/drain/cap from
//! `fairness_cursor`, honor both deadlines, and emit a `poll_cycle_outcome`.
//! Binding is not a serial preamble. After a decision the runner drops
//! binds; it does not await [`Observer::cancel`].
//!
//! Acknowledged anchors are applied at bind. Pending binds never mint a
//! provider cursor. Collection stops when representable bounds are exhausted.
//! Request `activation_ref` is not copied onto observed events.

use std::collections::btree_map::Entry;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use waitprims_core::{
    Anchor, ArmStatus, AuthnMode, CoverageArm, IdToken, OutcomeKind, PollCycleOutcome,
    PollCycleRequest, Registration, RegistrationSet, Result, Timestamp, ValidationError, WaitEvent,
};

use crate::cancel::Cancel;
use crate::clock::Clock;
use crate::observer::{BindHandle, Observation, Observer};
use crate::outcome::ResolvedStart;
use crate::race::{observation_is_terminal, FirstReady};

#[derive(Debug, Clone)]
enum ArmVisit {
    Events(Vec<WaitEvent>),
    Saturated(Vec<WaitEvent>),
    Idle,
    Overflow,
    Failed(String),
    Outage(String),
    CursorUncertain(String),
    Degraded(String),
    Deferred,
}

/// Byte weight of one event for poll-cycle bounds.
///
/// `payload_ref` is opaque; this crate does not fetch the referenced body.
/// `max_bytes` and [`CoverageArm::byte_count`] therefore count the observable
/// structured surface: the payload_ref token, the digest hex, and optional
/// media_type. They do not estimate remote payload size.
pub fn event_surface_bytes(event: &WaitEvent) -> u64 {
    let mut n = event.payload.payload_ref.as_str().len() as u64;
    n += event.payload.content_digest.value.len() as u64;
    if let Some(media_type) = &event.payload.media_type {
        n += media_type.len() as u64;
    }
    n
}

struct CollectBudget {
    max_events: u64,
    max_payload_refs: u64,
    max_bytes: u64,
    events: u64,
    payload_refs: u64,
    bytes: u64,
    per_reg_events: BTreeMap<String, u64>,
    per_reg_bytes: BTreeMap<String, u64>,
    reg_max_events: BTreeMap<String, u64>,
    reg_max_bytes: BTreeMap<String, u64>,
}

impl CollectBudget {
    fn new(set: &RegistrationSet, request: &PollCycleRequest) -> Self {
        let req_events = request
            .bound
            .as_ref()
            .and_then(|bound| bound.max_events)
            .unwrap_or(u64::MAX);
        let req_bytes = request
            .bound
            .as_ref()
            .and_then(|bound| bound.max_bytes)
            .unwrap_or(u64::MAX);
        let req_refs = request
            .bound
            .as_ref()
            .and_then(|bound| bound.max_payload_refs)
            .unwrap_or(u64::MAX);
        Self {
            max_events: set.aggregate_limits.max_events.min(req_events),
            max_payload_refs: req_refs,
            max_bytes: set.aggregate_limits.max_bytes.min(req_bytes),
            events: 0,
            payload_refs: 0,
            bytes: 0,
            per_reg_events: BTreeMap::new(),
            per_reg_bytes: BTreeMap::new(),
            reg_max_events: set
                .registrations
                .iter()
                .map(|reg| {
                    (
                        reg.registration_id.as_str().to_string(),
                        reg.bounds.max_events,
                    )
                })
                .collect(),
            reg_max_bytes: set
                .registrations
                .iter()
                .map(|reg| {
                    (
                        reg.registration_id.as_str().to_string(),
                        reg.bounds.max_bytes,
                    )
                })
                .collect(),
        }
    }

    fn room_for_another_event(&self) -> bool {
        self.events < self.max_events
            && self.payload_refs < self.max_payload_refs
            && self.bytes < self.max_bytes
    }

    fn room_for_registration(&self, registration_id: &str) -> bool {
        if !self.room_for_another_event() {
            return false;
        }
        let reg_events = self
            .per_reg_events
            .get(registration_id)
            .copied()
            .unwrap_or(0);
        let reg_bytes = self
            .per_reg_bytes
            .get(registration_id)
            .copied()
            .unwrap_or(0);
        let max_e = self
            .reg_max_events
            .get(registration_id)
            .copied()
            .unwrap_or(u64::MAX);
        let max_b = self
            .reg_max_bytes
            .get(registration_id)
            .copied()
            .unwrap_or(u64::MAX);
        reg_events < max_e && reg_bytes < max_b
    }

    fn exhausted(&self) -> bool {
        !self.room_for_another_event()
    }

    fn try_take(&mut self, event: &WaitEvent) -> bool {
        let rid = event.registration_id.as_str();
        if !self.room_for_registration(rid) {
            return false;
        }
        let weight = event_surface_bytes(event);
        let reg_bytes = self.per_reg_bytes.get(rid).copied().unwrap_or(0);
        let max_b = self.reg_max_bytes.get(rid).copied().unwrap_or(u64::MAX);
        if self.bytes.saturating_add(weight) > self.max_bytes {
            return false;
        }
        if reg_bytes.saturating_add(weight) > max_b {
            return false;
        }
        self.events = self.events.saturating_add(1);
        self.payload_refs = self.payload_refs.saturating_add(1);
        self.bytes = self.bytes.saturating_add(weight);
        *self.per_reg_events.entry(rid.to_string()).or_insert(0) += 1;
        *self.per_reg_bytes.entry(rid.to_string()).or_insert(0) += weight;
        true
    }
}

/// Resolve the cited registration set/revision and run one poll cycle.
///
/// Bind, next, cancel, and both deadlines share one race. A required
/// registration still pending bind at a terminal deadline is `failed`,
/// never a clean-complete `no_change` / `logical_deadman`. Fairness
/// leftover at `run_deadline` is `deferred`. Outage, cursor uncertainty,
/// and degradation on a required arm are never a clean complete.
/// Callers serialize [`waitprims_core::AgentWaitMessage::PollCycleOutcome`]
/// for the wire.
pub async fn run_poll_cycle<O, C>(
    set: &RegistrationSet,
    request: &PollCycleRequest,
    observer: &O,
    clock: &C,
    cancel: &Cancel,
) -> Result<PollCycleOutcome>
where
    O: Observer,
    C: Clock,
{
    resolve(set, request)?;
    run_loop(set, request, observer, clock, cancel).await
}

fn resolve(set: &RegistrationSet, request: &PollCycleRequest) -> Result<()> {
    if request.registration_set_ref.as_str() != set.message_id.as_str() {
        return Err(ValidationError::new("/registration_set_ref", "mismatch").into());
    }
    if request.registration_revision.as_str() != set.registration_revision.as_str() {
        return Err(ValidationError::new("/registration_revision", "revision_mismatch").into());
    }
    if request.waiter_id.as_str() != set.waiter_id.as_str() {
        return Err(ValidationError::new("/waiter_id", "mismatch").into());
    }
    for key in request.acknowledged_anchors.keys() {
        if !set
            .registrations
            .iter()
            .any(|registration| registration.registration_id.as_str() == key)
        {
            return Err(
                ValidationError::new("/acknowledged_anchors", "unknown_registration").into(),
            );
        }
    }
    Ok(())
}

fn bind_registration(registration: &Registration, request: &PollCycleRequest) -> Registration {
    let mut next = registration.clone();
    if let Some(ack) = request
        .acknowledged_anchors
        .get(registration.registration_id.as_str())
    {
        next.start_anchor = Some(ack.clone());
        next.baseline_policy = None;
    }
    next
}

async fn bind_one<O: Observer>(
    observer: &O,
    registration: Registration,
    request: &PollCycleRequest,
    resolved: &Mutex<Vec<ResolvedStart>>,
) -> Result<O::Bind> {
    let bind = observer.bind(&registration).await?;
    if let Some(expected) = request
        .acknowledged_anchors
        .get(registration.registration_id.as_str())
    {
        if bind.resolved_start() != expected {
            return Err(
                ValidationError::new("/acknowledged_anchors", "bind_start_mismatch").into(),
            );
        }
    }
    record_resolved(resolved, &bind);
    Ok(bind)
}

fn bind_is_terminal<B>(_: &B) -> bool {
    true
}

fn record_resolved<B: BindHandle>(resolved: &Mutex<Vec<ResolvedStart>>, bind: &B) {
    resolved
        .lock()
        .expect("resolved-start")
        .push(ResolvedStart {
            registration_id: bind.registration_id().clone(),
            start: bind.resolved_start().clone(),
        });
}

fn snapshot(resolved: &Mutex<Vec<ResolvedStart>>) -> Vec<ResolvedStart> {
    resolved.lock().expect("resolved-start").clone()
}

fn merge_starts(slots: &[ResolvedStart], resolved: &[ResolvedStart]) -> Vec<ResolvedStart> {
    let mut by_id = BTreeMap::new();
    for start in resolved.iter().chain(slots) {
        by_id.insert(start.registration_id.as_str().to_string(), start.clone());
    }
    by_id.into_values().collect()
}

fn starts_from_slots<B: BindHandle>(binds: &[Option<B>]) -> Vec<ResolvedStart> {
    binds
        .iter()
        .filter_map(|bind| {
            bind.as_ref().map(|bind| ResolvedStart {
                registration_id: bind.registration_id().clone(),
                start: bind.resolved_start().clone(),
            })
        })
        .collect()
}

fn remap_ready<T>(pending: &[usize], ready: Vec<(usize, T)>) -> Vec<(usize, T)> {
    ready
        .into_iter()
        .filter_map(|(local, value)| pending.get(local).copied().map(|idx| (idx, value)))
        .collect()
}

fn pending_refs<'a, B>(binds: &'a [Option<B>], pending: &[usize]) -> Vec<(usize, &'a B)> {
    pending
        .iter()
        .filter_map(|&idx| {
            binds
                .get(idx)
                .and_then(|bind| bind.as_ref())
                .map(|bind| (idx, bind))
        })
        .collect()
}

async fn observe_pending<O: Observer>(
    observer: &O,
    bound: &[Option<O::Bind>],
    pending: &[usize],
) -> Result<Vec<(usize, Observation)>> {
    if pending.is_empty() {
        std::future::pending::<()>().await;
    }
    let ready = FirstReady::new(
        pending
            .iter()
            .map(|&idx| observer.next(bound[idx].as_ref().expect("bound"))),
        observation_is_terminal,
    )
    .await?;
    Ok(remap_ready(pending, ready))
}

async fn bind_unbound<O: Observer>(
    observer: &O,
    set: &RegistrationSet,
    request: &PollCycleRequest,
    unbound: &[usize],
    resolved: &Arc<Mutex<Vec<ResolvedStart>>>,
) -> Result<Vec<(usize, O::Bind)>> {
    if unbound.is_empty() {
        std::future::pending::<()>().await;
    }
    let ready = FirstReady::new(
        unbound.iter().map(|&idx| {
            let resolved = Arc::clone(resolved);
            let registration = bind_registration(&set.registrations[idx], request);
            async move { bind_one(observer, registration, request, &resolved).await }
        }),
        bind_is_terminal,
    )
    .await?;
    Ok(remap_ready(unbound, ready))
}

fn occupied_refs<B>(binds: &[Option<B>]) -> Vec<(usize, &B)> {
    binds
        .iter()
        .enumerate()
        .filter_map(|(idx, bind)| bind.as_ref().map(|bind| (idx, bind)))
        .collect()
}

fn required_binding_complete(set: &RegistrationSet, resolved: &[ResolvedStart]) -> bool {
    set.registrations
        .iter()
        .filter(|registration| registration.required)
        .all(|registration| {
            resolved.iter().any(|start| {
                start.registration_id.as_str() == registration.registration_id.as_str()
            })
        })
}

fn fairness_order(set: &RegistrationSet, request: &PollCycleRequest) -> Vec<usize> {
    let n = set.registrations.len();
    if n == 0 {
        return Vec::new();
    }
    let start = fairness_start_index(set, request);
    (0..n).map(|i| (start + i) % n).collect()
}

async fn run_loop<O, C>(
    set: &RegistrationSet,
    request: &PollCycleRequest,
    observer: &O,
    clock: &C,
    cancel: &Cancel,
) -> Result<PollCycleOutcome>
where
    O: Observer,
    C: Clock,
{
    let mut bound: Vec<Option<O::Bind>> = set.registrations.iter().map(|_| None).collect();
    let mut any_bound = false;
    let resolved = Arc::new(Mutex::new(Vec::<ResolvedStart>::new()));
    let mut visits: BTreeMap<usize, ArmVisit> = BTreeMap::new();
    let mut budget = CollectBudget::new(set, request);
    let order = fairness_order(set, request);
    loop {
        if cancel.is_cancelled() {
            return finish(
                set,
                request,
                clock.now(),
                &snapshot(&resolved),
                &visits,
                Some("consumer_cancelled"),
            );
        }
        let starts = merge_starts(&starts_from_slots(&bound), &snapshot(&resolved));
        if deadline_reached(request, &clock.now()) {
            return decide_at_deadline(set, request, &clock.now(), &starts, &visits);
        }

        let wait_until = earliest_deadline(request);
        if any_bound {
            let pending: Vec<usize> = order
                .iter()
                .copied()
                .filter(|idx| bound[*idx].is_some() && !visits.contains_key(idx))
                .collect();
            if pending.is_empty() && visits.len() == set.registrations.len() {
                return assemble(set, request, clock.now(), &starts, &visits);
            }
            let unbound: Vec<usize> = order
                .iter()
                .copied()
                .filter(|idx| bound[*idx].is_none() && !visits.contains_key(idx))
                .collect();
            if pending.is_empty() && unbound.is_empty() {
                return assemble(set, request, clock.now(), &starts, &visits);
            }
            tokio::select! {
                biased;
                ready = observe_pending(observer, &bound, &pending) => {
                    let mut ready = ready?;
                    let indexed = pending_refs(&bound, &pending);
                    extend_ready_refs(observer, &indexed, &mut ready);
                    record_ready(observer, set, request, &bound, &mut budget, &mut visits, ready);
                    if visits.len() == set.registrations.len() {
                        return assemble(set, request, clock.now(), &starts, &visits);
                    }
                }
                ready = bind_unbound(observer, set, request, &unbound, &resolved) => {
                    for (idx, bind) in ready? {
                        bound[idx] = Some(bind);
                    }
                    let mut observations = Vec::new();
                    let indexed = occupied_refs(&bound);
                    extend_ready_refs(observer, &indexed, &mut observations);
                    record_ready(observer, set, request, &bound, &mut budget, &mut visits, observations);
                    if visits.len() == set.registrations.len() {
                        return assemble(
                            set,
                            request,
                            clock.now(),
                            &starts_from_slots(&bound),
                            &visits,
                        );
                    }
                }
                _ = cancel.cancelled() => {
                    return finish(
                        set,
                        request,
                        clock.now(),
                        &starts,
                        &visits,
                        Some("consumer_cancelled"),
                    );
                }
                _ = clock.sleep_until(wait_until) => {
                    let now = clock.now();
                    let mut ready = Vec::new();
                    let indexed = pending_refs(&bound, &pending);
                    extend_ready_refs(observer, &indexed, &mut ready);
                    record_ready(observer, set, request, &bound, &mut budget, &mut visits, ready);
                    return decide_at_deadline(set, request, &now, &starts, &visits);
                }
            }
            continue;
        }

        tokio::select! {
            biased;
            ready = bind_unbound(observer, set, request, &order, &resolved) => {
                for (idx, bind) in ready? {
                    bound[idx] = Some(bind);
                    any_bound = true;
                }
                let mut observations = Vec::new();
                let indexed = occupied_refs(&bound);
                extend_ready_refs(observer, &indexed, &mut observations);
                record_ready(observer, set, request, &bound, &mut budget, &mut visits, observations);
                if visits.len() == set.registrations.len() {
                    return assemble(
                        set,
                        request,
                        clock.now(),
                        &starts_from_slots(&bound),
                        &visits,
                    );
                }
            }
            _ = cancel.cancelled() => {
                return finish(
                    set,
                    request,
                    clock.now(),
                    &snapshot(&resolved),
                    &visits,
                    Some("consumer_cancelled"),
                );
            }
            _ = clock.sleep_until(wait_until) => {
                let now = clock.now();
                let starts = snapshot(&resolved);
                return decide_at_deadline(set, request, &now, &starts, &visits);
            }
        }
    }
}

fn extend_ready_refs<O: Observer>(
    observer: &O,
    binds: &[(usize, &O::Bind)],
    ready: &mut Vec<(usize, Observation)>,
) {
    for (idx, bind) in binds {
        if ready.iter().any(|(have, _)| have == idx) {
            continue;
        }
        if let Some(obs) = observer.poll_ready(bind) {
            ready.push((*idx, obs));
        }
    }
}

fn restore_observation<O: Observer>(observer: &O, bind: Option<&O::Bind>, obs: Observation) {
    if let Some(bind) = bind {
        observer.restore_ready(bind, obs);
    }
}

fn defer_ready<O: Observer>(observer: &O, bind: Option<&O::Bind>, obs: Observation) -> ArmVisit {
    restore_observation(observer, bind, obs);
    ArmVisit::Deferred
}

fn record_ready<O: Observer>(
    observer: &O,
    set: &RegistrationSet,
    request: &PollCycleRequest,
    binds: &[Option<O::Bind>],
    budget: &mut CollectBudget,
    visits: &mut BTreeMap<usize, ArmVisit>,
    ready: Vec<(usize, Observation)>,
) {
    let mut ready_map: BTreeMap<usize, Observation> = BTreeMap::new();
    for (idx, obs) in ready {
        match ready_map.entry(idx) {
            Entry::Vacant(slot) => {
                slot.insert(obs);
            }
            Entry::Occupied(_) => {
                restore_observation(observer, binds.get(idx).and_then(|bind| bind.as_ref()), obs);
            }
        }
    }
    for idx in fairness_order(set, request) {
        if visits.contains_key(&idx) {
            continue;
        }
        let Some(obs) = ready_map.remove(&idx) else {
            continue;
        };
        let bind = binds.get(idx).and_then(|bind| bind.as_ref());
        if budget.exhausted() && matches!(obs, Observation::Event(_)) {
            visits.insert(idx, defer_ready(observer, bind, obs));
            continue;
        }
        visits.insert(idx, visit_from(observer, bind, budget, obs));
    }
    for (idx, obs) in ready_map {
        restore_observation(observer, binds.get(idx).and_then(|bind| bind.as_ref()), obs);
    }
}

fn visit_from<O: Observer>(
    observer: &O,
    bind: Option<&O::Bind>,
    budget: &mut CollectBudget,
    obs: Observation,
) -> ArmVisit {
    match obs {
        Observation::Event(event) => {
            let event = *event;
            if !budget.try_take(&event) {
                return defer_ready(observer, bind, Observation::Event(Box::new(event)));
            }
            let mut events = vec![event];
            let truncated = bind
                .map(|bind| drain_ready(observer, bind, budget, &mut events))
                .unwrap_or(false);
            if truncated {
                ArmVisit::Saturated(events)
            } else {
                ArmVisit::Events(events)
            }
        }
        Observation::Idle => ArmVisit::Idle,
        Observation::Overflow => ArmVisit::Overflow,
        Observation::Failed { reason_code } => ArmVisit::Failed(reason_code.as_str().to_string()),
        Observation::Outage { reason_code } => ArmVisit::Outage(reason_code.as_str().to_string()),
        Observation::CursorUncertain { reason_code } => {
            ArmVisit::CursorUncertain(reason_code.as_str().to_string())
        }
        Observation::Degraded { reason_code } => {
            ArmVisit::Degraded(reason_code.as_str().to_string())
        }
    }
}

fn drain_ready<O: Observer>(
    observer: &O,
    bind: &O::Bind,
    budget: &mut CollectBudget,
    events: &mut Vec<WaitEvent>,
) -> bool {
    let rid = bind.registration_id().as_str();
    if !budget.room_for_registration(rid) {
        return true;
    }
    loop {
        if !budget.room_for_registration(rid) {
            return true;
        }
        let Some(obs) = observer.poll_ready(bind) else {
            return false;
        };
        match obs {
            Observation::Event(event) => {
                let event = *event;
                if !budget.try_take(&event) {
                    restore_observation(observer, Some(bind), Observation::Event(Box::new(event)));
                    return true;
                }
                events.push(event);
            }
            Observation::Overflow => return true,
            other => {
                restore_observation(observer, Some(bind), other);
                return false;
            }
        }
    }
}

fn deadline_reached(request: &PollCycleRequest, now: &Timestamp) -> bool {
    now >= &request.logical_deadline || now >= &request.run_deadline
}

fn earliest_deadline(request: &PollCycleRequest) -> &Timestamp {
    if request.run_deadline < request.logical_deadline {
        &request.run_deadline
    } else {
        &request.logical_deadline
    }
}

fn decide_at_deadline(
    set: &RegistrationSet,
    request: &PollCycleRequest,
    now: &Timestamp,
    resolved: &[ResolvedStart],
    visits: &BTreeMap<usize, ArmVisit>,
) -> Result<PollCycleOutcome> {
    if !required_binding_complete(set, resolved) {
        return finish(
            set,
            request,
            now.clone(),
            resolved,
            visits,
            Some("required_bind_pending"),
        );
    }
    let mut visits = visits.clone();
    let saw_events = visits.values().any(|visit| {
        matches!(
            visit,
            ArmVisit::Events(events) | ArmVisit::Saturated(events) if !events.is_empty()
        )
    });
    for idx in fairness_order(set, request) {
        visits.entry(idx).or_insert_with(|| {
            if saw_events {
                ArmVisit::Deferred
            } else {
                ArmVisit::Idle
            }
        });
    }
    assemble(set, request, now.clone(), resolved, &visits)
}

fn finish(
    set: &RegistrationSet,
    request: &PollCycleRequest,
    now: Timestamp,
    resolved: &[ResolvedStart],
    visits: &BTreeMap<usize, ArmVisit>,
    force_reason: Option<&str>,
) -> Result<PollCycleOutcome> {
    if let Some(reason) = force_reason {
        let kind = if reason == "consumer_cancelled" {
            OutcomeKind::Cancelled
        } else {
            OutcomeKind::Failed
        };
        return Ok(apply_posture(
            set,
            request,
            build(set, request, now, resolved, visits, kind, Some(reason))?,
        ));
    }
    assemble(set, request, now, resolved, visits)
}

fn assemble(
    set: &RegistrationSet,
    request: &PollCycleRequest,
    now: Timestamp,
    resolved: &[ResolvedStart],
    visits: &BTreeMap<usize, ArmVisit>,
) -> Result<PollCycleOutcome> {
    let outcome = decide_kind(set, request, now, resolved, visits)?;
    Ok(apply_posture(set, request, outcome))
}

fn decide_kind(
    set: &RegistrationSet,
    request: &PollCycleRequest,
    now: Timestamp,
    resolved: &[ResolvedStart],
    visits: &BTreeMap<usize, ArmVisit>,
) -> Result<PollCycleOutcome> {
    let mut events = Vec::new();
    let mut overflow = false;
    let mut truncated = false;
    let mut failed = None;
    let mut dirty = false;
    let mut deferred_required = false;
    for idx in fairness_order(set, request) {
        let registration = &set.registrations[idx];
        match visits.get(&idx) {
            Some(ArmVisit::Events(found)) => events.extend(found.iter().cloned()),
            Some(ArmVisit::Saturated(found)) => {
                events.extend(found.iter().cloned());
                truncated = true;
            }
            Some(ArmVisit::Overflow) => overflow = true,
            Some(ArmVisit::Failed(reason)) => failed = Some(reason.clone()),
            Some(ArmVisit::Outage(_))
            | Some(ArmVisit::CursorUncertain(_))
            | Some(ArmVisit::Degraded(_)) => {
                if registration.required {
                    dirty = true;
                }
            }
            Some(ArmVisit::Deferred) => {
                if registration.required {
                    deferred_required = true;
                }
            }
            Some(ArmVisit::Idle) | None => {}
        }
    }

    let at_logical = now >= request.logical_deadline;

    if overflow && events.is_empty() {
        return build(
            set,
            request,
            now,
            resolved,
            visits,
            OutcomeKind::Failed,
            Some("buffer_overflow"),
        );
    }
    if overflow {
        return build(
            set,
            request,
            now,
            resolved,
            visits,
            OutcomeKind::Partial,
            None,
        );
    }
    if dirty {
        return build(
            set,
            request,
            now,
            resolved,
            visits,
            OutcomeKind::CoverageDegraded,
            None,
        );
    }
    if let Some(reason) = failed {
        if events.is_empty() {
            return build(
                set,
                request,
                now,
                resolved,
                visits,
                OutcomeKind::Failed,
                Some(&reason),
            );
        }
        return build(
            set,
            request,
            now,
            resolved,
            visits,
            OutcomeKind::CoverageDegraded,
            None,
        );
    }
    if !events.is_empty() {
        let kind = if deferred_required || truncated {
            OutcomeKind::Partial
        } else {
            OutcomeKind::Events
        };
        return build(set, request, now, resolved, visits, kind, None);
    }
    if truncated {
        return build(
            set,
            request,
            now,
            resolved,
            visits,
            OutcomeKind::Partial,
            None,
        );
    }
    if deferred_required && !at_logical {
        return build(
            set,
            request,
            now,
            resolved,
            visits,
            OutcomeKind::Partial,
            None,
        );
    }
    if at_logical {
        return build(
            set,
            request,
            now,
            resolved,
            visits,
            OutcomeKind::LogicalDeadman,
            None,
        );
    }
    build(
        set,
        request,
        now,
        resolved,
        visits,
        OutcomeKind::NoChange,
        None,
    )
}

fn apply_posture(
    set: &RegistrationSet,
    request: &PollCycleRequest,
    outcome: PollCycleOutcome,
) -> PollCycleOutcome {
    if matches!(
        outcome.outcome_kind,
        OutcomeKind::Cancelled
            | OutcomeKind::Failed
            | OutcomeKind::Refused
            | OutcomeKind::ReauthenticationRequired
    ) {
        return outcome;
    }
    if set
        .registrations
        .iter()
        .any(|reg| outcome.completed_at > reg.lease_expires_at)
    {
        return rebuild_reason(
            set,
            request,
            outcome,
            OutcomeKind::ReauthenticationRequired,
            "lease_expired",
        );
    }
    if set.authn_mode == AuthnMode::Required && request.verification_receipt_ref.is_none() {
        return rebuild_reason(
            set,
            request,
            outcome,
            OutcomeKind::Refused,
            "authn_required",
        );
    }
    outcome
}

fn rebuild_reason(
    set: &RegistrationSet,
    request: &PollCycleRequest,
    mut outcome: PollCycleOutcome,
    kind: OutcomeKind,
    reason: &str,
) -> PollCycleOutcome {
    outcome.outcome_kind = kind;
    outcome.reason_code = Some(IdToken::new(reason));
    outcome.events.clear();
    outcome.coverage_complete = false;
    for arm in &mut outcome.arms {
        if arm.status == ArmStatus::Events {
            arm.status = ArmStatus::NoChange;
            arm.event_count = 0;
            arm.byte_count = 0;
        }
    }
    outcome.retained_events = empty_event_map(&outcome);
    let _ = (set, request);
    outcome
}

fn build(
    set: &RegistrationSet,
    request: &PollCycleRequest,
    completed_at: Timestamp,
    resolved: &[ResolvedStart],
    visits: &BTreeMap<usize, ArmVisit>,
    kind: OutcomeKind,
    reason_code: Option<&str>,
) -> Result<PollCycleOutcome> {
    let mut events = Vec::new();
    let mut arms = Vec::new();
    let mut retained_through = BTreeMap::new();
    let mut retained_events = BTreeMap::new();
    let mut proposed_next_anchors = BTreeMap::new();
    let mut required_index = 0usize;

    for idx in 0..set.registrations.len() {
        let registration = &set.registrations[idx];
        let arm_id = arm_id_for(request, registration, &mut required_index);
        let Some(start) = honest_start(resolved, request, registration) else {
            continue;
        };
        let visit = visits.get(&idx);
        let (status, degraded, reason, arm_events) = visit_status(visit);
        let proposed = arm_events
            .last()
            .map(|event| event.proposed_next_anchor.clone())
            .unwrap_or_else(|| start.clone());
        let retain_events = matches!(kind, OutcomeKind::Events | OutcomeKind::Partial)
            && status == ArmStatus::Events
            && !arm_events.is_empty();
        let retained_ids: Vec<IdToken> = if retain_events {
            arm_events
                .iter()
                .map(|event| event.event_id.clone())
                .collect()
        } else {
            Vec::new()
        };
        let retained_cursor = if retain_events {
            proposed.clone()
        } else {
            start.clone()
        };
        if matches!(kind, OutcomeKind::Events | OutcomeKind::Partial) {
            events.extend(arm_events.iter().cloned());
        }
        let byte_count = arm_events.iter().map(event_surface_bytes).sum();
        let mut arm = CoverageArm {
            arm_id,
            registration_id: registration.registration_id.clone(),
            required: registration.required,
            status,
            degraded,
            start_anchor: start.clone(),
            proposed_next_anchor: proposed.clone(),
            event_count: arm_events.len() as u64,
            byte_count,
            reason_code: None,
        };
        if let Some(code) = reason {
            arm.reason_code = Some(IdToken::new(code));
        } else if degraded || matches!(status, ArmStatus::Outage | ArmStatus::CursorUncertain) {
            arm.reason_code = Some(IdToken::new("arm_dirty"));
        }
        arms.push(arm);
        retained_through.insert(
            registration.registration_id.as_str().to_string(),
            retained_cursor,
        );
        retained_events.insert(
            registration.registration_id.as_str().to_string(),
            retained_ids,
        );
        proposed_next_anchors.insert(registration.registration_id.as_str().to_string(), proposed);
    }

    if arms.is_empty() {
        return Err(ValidationError::new("/arms", "unresolved_start").into());
    }

    if matches!(
        kind,
        OutcomeKind::Cancelled
            | OutcomeKind::Failed
            | OutcomeKind::Refused
            | OutcomeKind::ReauthenticationRequired
    ) {
        events.clear();
        for arm in &mut arms {
            if arm.status == ArmStatus::Events {
                arm.status = ArmStatus::NoChange;
                arm.event_count = 0;
                arm.byte_count = 0;
            }
        }
        retained_events = empty_event_map_from_arms(&arms);
    }

    let coverage_complete = coverage_is_complete(request, &arms, kind);
    let acknowledged = if request.acknowledged_anchors.is_empty() {
        None
    } else {
        Some(request.acknowledged_anchors.clone())
    };

    Ok(PollCycleOutcome {
        capabilities: request.capabilities.clone(),
        message_id: IdToken::new(format!("{}:outcome", request.message_id.as_str())),
        correlation_id: request.correlation_id.clone(),
        created_at: completed_at.clone(),
        actor_ref: request.actor_ref.clone(),
        causation_id: Some(request.message_id.clone()),
        grant_ref: request.grant_ref.clone(),
        verification_receipt_ref: request.verification_receipt_ref.clone(),
        policy_decision_ref: request.policy_decision_ref.clone(),
        waiter_id: request.waiter_id.clone(),
        request_ref: request.message_id.clone(),
        completed_at,
        logical_deadline: request.logical_deadline.clone(),
        outcome_kind: kind,
        events,
        coverage_complete,
        arms,
        retained_through,
        retained_events,
        proposed_next_anchors,
        fairness_cursor: request.fairness_cursor.clone(),
        next_fairness_cursor: next_fairness_cursor(set, request),
        acknowledged_anchors: acknowledged,
        bound: request.bound.clone(),
        reason_code: reason_code.map(IdToken::new),
    })
}

fn visit_status(visit: Option<&ArmVisit>) -> (ArmStatus, bool, Option<String>, Vec<WaitEvent>) {
    match visit {
        Some(ArmVisit::Events(events)) => (ArmStatus::Events, false, None, events.clone()),
        Some(ArmVisit::Saturated(events)) => (
            ArmStatus::Events,
            false,
            Some("bound_exhausted".to_string()),
            events.clone(),
        ),
        Some(ArmVisit::Idle) => (ArmStatus::NoChange, false, None, Vec::new()),
        Some(ArmVisit::Overflow) => (
            ArmStatus::Observed,
            false,
            Some("buffer_overflow".to_string()),
            Vec::new(),
        ),
        Some(ArmVisit::Failed(reason)) => {
            (ArmStatus::Observed, false, Some(reason.clone()), Vec::new())
        }
        Some(ArmVisit::Outage(reason)) => {
            (ArmStatus::Outage, false, Some(reason.clone()), Vec::new())
        }
        Some(ArmVisit::CursorUncertain(reason)) => (
            ArmStatus::CursorUncertain,
            false,
            Some(reason.clone()),
            Vec::new(),
        ),
        Some(ArmVisit::Degraded(reason)) => {
            (ArmStatus::Observed, true, Some(reason.clone()), Vec::new())
        }
        Some(ArmVisit::Deferred) | None => (ArmStatus::Deferred, false, None, Vec::new()),
    }
}

fn coverage_is_complete(
    request: &PollCycleRequest,
    arms: &[CoverageArm],
    kind: OutcomeKind,
) -> bool {
    if matches!(
        kind,
        OutcomeKind::Partial
            | OutcomeKind::CoverageDegraded
            | OutcomeKind::Cancelled
            | OutcomeKind::Failed
            | OutcomeKind::Refused
            | OutcomeKind::ReauthenticationRequired
    ) {
        return false;
    }
    request.required_arms.iter().all(|required| {
        arms.iter().any(|arm| {
            arm.arm_id.as_str() == required.as_str()
                && !arm.degraded
                && !matches!(arm.status, ArmStatus::Outage | ArmStatus::CursorUncertain)
        })
    })
}

fn honest_start(
    resolved: &[ResolvedStart],
    request: &PollCycleRequest,
    registration: &Registration,
) -> Option<Anchor> {
    if let Some(start) = resolved
        .iter()
        .find(|item| item.registration_id.as_str() == registration.registration_id.as_str())
    {
        return Some(start.start.clone());
    }
    if let Some(ack) = request
        .acknowledged_anchors
        .get(registration.registration_id.as_str())
    {
        return Some(ack.clone());
    }
    registration.start_anchor.clone()
}

fn arm_id_for(
    request: &PollCycleRequest,
    registration: &Registration,
    required_index: &mut usize,
) -> IdToken {
    if registration.required {
        let idx = *required_index;
        *required_index += 1;
        if let Some(id) = request.required_arms.get(idx) {
            return id.clone();
        }
    }
    derived_arm_id(registration)
}

fn derived_arm_id(registration: &Registration) -> IdToken {
    let rest = registration
        .registration_id
        .as_str()
        .strip_prefix("reg:")
        .unwrap_or(registration.registration_id.as_str());
    IdToken::new(format!("arm:{rest}"))
}

fn next_fairness_cursor(set: &RegistrationSet, request: &PollCycleRequest) -> IdToken {
    if set.registrations.is_empty() {
        return request.fairness_cursor.clone();
    }
    let start = fairness_start_index(set, request);
    let next = (start + 1) % set.registrations.len();
    let mut required_index = 0usize;
    let mut chosen = None;
    for (idx, registration) in set.registrations.iter().enumerate() {
        let arm_id = arm_id_for(request, registration, &mut required_index);
        if idx == next {
            chosen = Some(arm_id);
            break;
        }
    }
    let arm_id = chosen.unwrap_or_else(|| derived_arm_id(&set.registrations[next]));
    if arm_id.as_str().starts_with("fair:") {
        arm_id
    } else {
        IdToken::new(format!("fair:{}", arm_id.as_str()))
    }
}

fn fairness_start_index(set: &RegistrationSet, request: &PollCycleRequest) -> usize {
    let cursor = request.fairness_cursor.as_str();
    if cursor == "fair:start" {
        return 0;
    }
    let mut required_index = 0usize;
    for (idx, registration) in set.registrations.iter().enumerate() {
        let arm_id = arm_id_for(request, registration, &mut required_index);
        let fair = format!("fair:{}", arm_id.as_str());
        if cursor == arm_id.as_str()
            || cursor == fair
            || cursor == registration.registration_id.as_str()
            || cursor == format!("fair:{}", registration.registration_id.as_str())
        {
            return idx;
        }
    }
    0
}

fn empty_event_map(outcome: &PollCycleOutcome) -> BTreeMap<String, Vec<IdToken>> {
    outcome
        .arms
        .iter()
        .map(|arm| (arm.registration_id.as_str().to_string(), Vec::new()))
        .collect()
}

fn empty_event_map_from_arms(arms: &[CoverageArm]) -> BTreeMap<String, Vec<IdToken>> {
    arms.iter()
        .map(|arm| (arm.registration_id.as_str().to_string(), Vec::new()))
        .collect()
}
