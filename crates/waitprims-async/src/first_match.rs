//! Live first-match runner.
//!
//! # Tie rule
//!
//! When two or more arms accept an observation at the same logical instant,
//! the winner is the arm that appears first in
//! `registration_set.registrations`. That order is the only tie break.
//! Same-instant losers are not dropped: they go through
//! [`Observer::restore_ready`] so a later wait can replay them.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use waitprims_core::{
    AuthnMode, LiveWaitOutcome, LiveWaitRequest, OutcomeKind, Registration, RegistrationSet,
    Result, Timestamp, ValidationError,
};

use crate::cancel::Cancel;
use crate::clock::Clock;
use crate::observer::{BindHandle, Observation, Observer};
use crate::outcome::{self, ResolvedStart};
use crate::race::{
    bound_observation_is_terminal, observation_is_replayable, observation_is_terminal, FirstReady,
};

/// Documented deterministic tie rule for same-instant accepted observations.
pub const TIE_RULE: &str =
    "same-instant winner is the earliest arm in registration_set.registrations";

const BACKOFF_MS: &[u64] = &[50, 100, 200, 400, 800, 1000];

/// Resolve the cited registration set/revision and first-match an observer.
///
/// Bind, next, cancel, and both deadlines share one race. Binding is not a
/// serial preamble. A `baseline_policy` start is resolved to an exclusive
/// provider cursor at bind and kept for coverage. A required registration
/// still pending bind at a terminal deadline is `failed`, never a
/// clean-complete `no_change` / `logical_deadman`. After a decision the
/// runner restores consumed non-winner observations, then drops binds; it
/// does not await [`Observer::cancel`]. Emits a `live_wait_outcome` body.
/// Callers serialize
/// [`waitprims_core::AgentWaitMessage::LiveWaitOutcome`] for the wire.
/// Observed events keep their own optional `delivery_ref` / `activation_ref`;
/// a match does not invent deliver/activate evidence.
pub async fn run_first_match<O, C>(
    set: &RegistrationSet,
    request: &LiveWaitRequest,
    observer: &O,
    clock: &C,
    cancel: &Cancel,
) -> Result<LiveWaitOutcome>
where
    O: Observer,
    C: Clock,
{
    resolve(set, request)?;
    run_loop(set, request, observer, clock, cancel).await
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

async fn bind_then_observe<O: Observer>(
    observer: &O,
    registration: &Registration,
    resolved: &Mutex<Vec<ResolvedStart>>,
) -> Result<(O::Bind, Observation)> {
    let bind = observer.bind(registration).await?;
    record_resolved(resolved, &bind);
    let observation = observer.next(&bind).await?;
    Ok((bind, observation))
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

fn starts_from_binds<B: BindHandle>(binds: &[B]) -> Vec<ResolvedStart> {
    binds
        .iter()
        .map(|bind| ResolvedStart {
            registration_id: bind.registration_id().clone(),
            start: bind.resolved_start().clone(),
        })
        .collect()
}

fn starts_from_indexed<B: BindHandle>(binds: &[(usize, B)]) -> Vec<ResolvedStart> {
    binds
        .iter()
        .map(|(_, bind)| ResolvedStart {
            registration_id: bind.registration_id().clone(),
            start: bind.resolved_start().clone(),
        })
        .collect()
}

fn indexed_binds<B>(binds: &[B]) -> Vec<(usize, &B)> {
    binds.iter().enumerate().collect()
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

async fn run_loop<O, C>(
    set: &RegistrationSet,
    request: &LiveWaitRequest,
    observer: &O,
    clock: &C,
    cancel: &Cancel,
) -> Result<LiveWaitOutcome>
where
    O: Observer,
    C: Clock,
{
    let mut backoff_step = 0usize;
    let mut bound: Option<Vec<O::Bind>> = None;
    let resolved = Arc::new(Mutex::new(Vec::<ResolvedStart>::new()));
    loop {
        if cancel.is_cancelled() {
            return Ok(finish(
                set,
                request,
                clock.now(),
                outcome::cancelled(request, clock.now()),
            ));
        }
        let starts = bound
            .as_ref()
            .map(|binds| starts_from_binds(binds))
            .unwrap_or_else(|| snapshot(&resolved));
        if let Some(done) = terminal_deadline(set, request, &clock.now(), &starts) {
            return Ok(finish(set, request, clock.now(), done));
        }

        let wait_until = earliest_deadline(request);
        if let Some(binds) = bound.as_ref() {
            let starts = starts_from_binds(binds);
            let mut race = FirstReady::new(
                binds.iter().map(|bind| observer.next(bind)),
                observation_is_terminal,
            );
            tokio::select! {
                biased;
                collected = &mut race => {
                    let indexed = indexed_binds(binds);
                    if let Some(err) = collected.error {
                        restore_owned(observer, &indexed, collected.ready)?;
                        return Err(err);
                    }
                    let mut ready = collected.ready;
                    extend_ready_refs(observer, &indexed, &mut ready);
                    if let Some(outcome) = decide_and_restore(
                        observer,
                        set,
                        request,
                        &clock.now(),
                        &indexed,
                        ready,
                        &starts,
                    )? {
                        return Ok(finish(set, request, clock.now(), outcome));
                    }
                    backoff_step = backoff_step.saturating_add(1);
                    sleep_backoff(clock, cancel, request, backoff_step).await;
                    if cancel.is_cancelled() {
                        return Ok(finish(
                            set,
                            request,
                            clock.now(),
                            outcome::cancelled(request, clock.now()),
                        ));
                    }
                }
                _ = cancel.cancelled() => {
                    let leftover = race.take_ready();
                    restore_owned(observer, &indexed_binds(binds), leftover)?;
                    return Ok(finish(
                        set,
                        request,
                        clock.now(),
                        outcome::cancelled(request, clock.now()),
                    ));
                }
                _ = clock.sleep_until(wait_until) => {
                    let now = clock.now();
                    let leftover = race.take_ready();
                    restore_owned(observer, &indexed_binds(binds), leftover)?;
                    let mut ready = Vec::new();
                    let indexed = indexed_binds(binds);
                    extend_ready_refs(observer, &indexed, &mut ready);
                    if let Some(outcome) = decide_and_restore(
                        observer,
                        set,
                        request,
                        &now,
                        &indexed,
                        ready,
                        &starts,
                    )? {
                        return Ok(finish(set, request, now, outcome));
                    }
                    if let Some(done) = terminal_deadline(set, request, &now, &starts) {
                        return Ok(finish(set, request, now, done));
                    }
                }
            }
            continue;
        }

        let mut race = FirstReady::new(
            set.registrations.iter().map(|registration| {
                let resolved = Arc::clone(&resolved);
                async move { bind_then_observe(observer, registration, &resolved).await }
            }),
            bound_observation_is_terminal,
        );
        tokio::select! {
            biased;
            collected = &mut race => {
                if let Some(err) = collected.error {
                    restore_bound_pairs(observer, collected.ready)?;
                    return Err(err);
                }
                let mut indexed_binds = Vec::new();
                let mut observations = Vec::new();
                for (idx, (bind, observation)) in collected.ready {
                    observations.push((idx, observation));
                    indexed_binds.push((idx, bind));
                }
                let starts = starts_from_indexed(&indexed_binds);
                {
                    let bind_refs: Vec<(usize, &O::Bind)> = indexed_binds
                        .iter()
                        .map(|(idx, bind)| (*idx, bind))
                        .collect();
                    extend_ready_refs(observer, &bind_refs, &mut observations);
                    if let Some(outcome) = decide_and_restore(
                        observer,
                        set,
                        request,
                        &clock.now(),
                        &bind_refs,
                        observations,
                        &starts,
                    )? {
                        return Ok(finish(set, request, clock.now(), outcome));
                    }
                }
                indexed_binds.sort_by_key(|(idx, _)| *idx);
                let held: Vec<O::Bind> = indexed_binds.into_iter().map(|(_, bind)| bind).collect();
                bound = Some(held);
                backoff_step = backoff_step.saturating_add(1);
                sleep_backoff(clock, cancel, request, backoff_step).await;
            }
            _ = cancel.cancelled() => {
                restore_bound_pairs(observer, race.take_ready())?;
                return Ok(finish(
                    set,
                    request,
                    clock.now(),
                    outcome::cancelled(request, clock.now()),
                ));
            }
            _ = clock.sleep_until(wait_until) => {
                restore_bound_pairs(observer, race.take_ready())?;
                let now = clock.now();
                let starts = snapshot(&resolved);
                if let Some(done) = terminal_deadline(set, request, &now, &starts) {
                    return Ok(finish(set, request, now, done));
                }
            }
        }
    }
}

async fn sleep_backoff<C: Clock>(
    clock: &C,
    cancel: &Cancel,
    request: &LiveWaitRequest,
    step: usize,
) {
    let sleep_to = backoff_deadline(clock, request, step);
    tokio::select! {
        biased;
        _ = cancel.cancelled() => {}
        _ = clock.sleep_until(&sleep_to) => {}
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

fn event_winner_idx(ready: &[(usize, Observation)]) -> Option<usize> {
    ready
        .iter()
        .filter_map(|(idx, obs)| match obs {
            Observation::Event(_) => Some(*idx),
            _ => None,
        })
        .min()
}

fn restore_owned<O: Observer>(
    observer: &O,
    binds: &[(usize, &O::Bind)],
    ready: Vec<(usize, Observation)>,
) -> Result<()> {
    for (idx, obs) in ready {
        if !observation_is_replayable(&obs) {
            continue;
        }
        if let Some((_, bind)) = binds.iter().find(|(have, _)| *have == idx) {
            observer.restore_ready(bind, obs)?;
        }
    }
    Ok(())
}

fn restore_bound_pairs<O: Observer>(
    observer: &O,
    ready: Vec<(usize, (O::Bind, Observation))>,
) -> Result<()> {
    for (_, (bind, obs)) in ready {
        if observation_is_replayable(&obs) {
            observer.restore_ready(&bind, obs)?;
        }
    }
    Ok(())
}

fn restore_losers<O: Observer>(
    observer: &O,
    binds: &[(usize, &O::Bind)],
    ready: Vec<(usize, Observation)>,
) -> Result<()> {
    let winner = event_winner_idx(&ready);
    for (idx, obs) in ready {
        if Some(idx) == winner || !observation_is_replayable(&obs) {
            continue;
        }
        if let Some((_, bind)) = binds.iter().find(|(have, _)| *have == idx) {
            observer.restore_ready(bind, obs)?;
        }
    }
    Ok(())
}

fn decide_and_restore<O: Observer>(
    observer: &O,
    set: &RegistrationSet,
    request: &LiveWaitRequest,
    now: &Timestamp,
    binds: &[(usize, &O::Bind)],
    ready: Vec<(usize, Observation)>,
    resolved: &[ResolvedStart],
) -> Result<Option<LiveWaitOutcome>> {
    let Some(outcome) = decide(set, request, now, &ready, resolved) else {
        return Ok(None);
    };
    if let Some(rejected) = posture_reject(set, request, now) {
        restore_owned(observer, binds, ready)?;
        return Ok(Some(rejected));
    }
    restore_losers(observer, binds, ready)?;
    Ok(Some(outcome))
}

fn decide(
    set: &RegistrationSet,
    request: &LiveWaitRequest,
    now: &Timestamp,
    ready: &[(usize, Observation)],
    resolved: &[ResolvedStart],
) -> Option<LiveWaitOutcome> {
    let mut events = Vec::new();
    let mut overflow = false;
    let mut failed = None;
    for (idx, obs) in ready {
        match obs {
            Observation::Event(event) => events.push((*idx, event.as_ref().clone())),
            Observation::Overflow => overflow = true,
            Observation::Failed { reason_code }
            | Observation::Outage { reason_code }
            | Observation::CursorUncertain { reason_code }
            | Observation::Degraded { reason_code } => {
                failed = Some(reason_code.as_str().to_string());
            }
            Observation::Idle => {}
        }
    }

    if overflow {
        let winner = events
            .into_iter()
            .min_by_key(|(idx, _)| *idx)
            .map(|(_, event)| event);
        return Some(if winner.is_some() {
            outcome::partial(set, request, now.clone(), winner, resolved)
        } else {
            outcome::failed(request, now.clone(), "buffer_overflow")
        });
    }

    if let Some(reason) = failed {
        if events.is_empty() {
            return Some(outcome::failed(request, now.clone(), &reason));
        }
    }

    let (_, event) = events.into_iter().min_by_key(|(idx, _)| *idx)?;
    Some(outcome::events(request, now.clone(), event))
}

fn terminal_deadline(
    set: &RegistrationSet,
    request: &LiveWaitRequest,
    now: &Timestamp,
    resolved: &[ResolvedStart],
) -> Option<LiveWaitOutcome> {
    let at_logical = now >= &request.logical_deadline;
    let at_run = now >= &request.run_deadline;
    if !at_logical && !at_run {
        return None;
    }
    if !required_binding_complete(set, resolved) {
        return Some(outcome::failed(
            request,
            now.clone(),
            "required_bind_pending",
        ));
    }
    if at_logical {
        return Some(outcome::logical_deadman(
            set,
            request,
            now.clone(),
            resolved,
        ));
    }
    Some(outcome::no_change(set, request, now.clone(), resolved))
}

fn earliest_deadline(request: &LiveWaitRequest) -> &Timestamp {
    if request.run_deadline < request.logical_deadline {
        &request.run_deadline
    } else {
        &request.logical_deadline
    }
}

fn backoff_deadline<C: Clock>(clock: &C, request: &LiveWaitRequest, step: usize) -> Timestamp {
    let idx = step.saturating_sub(1).min(BACKOFF_MS.len() - 1);
    let delay = Duration::from_millis(BACKOFF_MS[idx]);
    let wake = clock.now().saturating_add(delay);
    let cap = earliest_deadline(request).clone();
    if wake < cap {
        wake
    } else {
        cap
    }
}

fn posture_reject(
    set: &RegistrationSet,
    request: &LiveWaitRequest,
    now: &Timestamp,
) -> Option<LiveWaitOutcome> {
    if set
        .registrations
        .iter()
        .any(|reg| now > &reg.lease_expires_at)
    {
        return Some(outcome::reauthentication_required(
            request,
            now.clone(),
            "lease_expired",
        ));
    }
    if set.authn_mode == AuthnMode::Required && request.verification_receipt_ref.is_none() {
        return Some(outcome::refused(request, now.clone(), "authn_required"));
    }
    None
}

fn finish(
    set: &RegistrationSet,
    request: &LiveWaitRequest,
    now: Timestamp,
    outcome: LiveWaitOutcome,
) -> LiveWaitOutcome {
    if matches!(
        outcome.outcome_kind,
        OutcomeKind::Cancelled
            | OutcomeKind::Failed
            | OutcomeKind::Refused
            | OutcomeKind::ReauthenticationRequired
    ) {
        return outcome;
    }
    if let Some(rejected) = posture_reject(set, request, &now) {
        return rejected;
    }
    outcome
}
