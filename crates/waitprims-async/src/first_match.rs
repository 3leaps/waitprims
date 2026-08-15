//! Live first-match runner.
//!
//! # Tie rule
//!
//! When two or more arms accept an observation at the same logical instant,
//! the winner is the arm that appears first in
//! `registration_set.registrations`. That order is the only tie break.

use std::time::Duration;

use waitprims_core::{
    AuthnMode, LiveWaitOutcome, LiveWaitRequest, OutcomeKind, Registration, RegistrationSet,
    Result, Timestamp, ValidationError,
};

use crate::cancel::Cancel;
use crate::clock::Clock;
use crate::observer::{Observation, Observer};
use crate::outcome;
use crate::race::{bound_observation_is_terminal, observation_is_terminal, FirstReady};

/// Documented deterministic tie rule for same-instant accepted observations.
pub const TIE_RULE: &str =
    "same-instant winner is the earliest arm in registration_set.registrations";

const BACKOFF_MS: &[u64] = &[50, 100, 200, 400, 800, 1000];

/// Resolve the cited registration set/revision and first-match an observer.
///
/// Bind, next, cancel, and both deadlines share one race. Binding is not a
/// serial preamble. Emits a `live_wait_outcome` body. Callers serialize
/// [`waitprims_core::AgentWaitMessage::LiveWaitOutcome`] for the wire.
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
) -> Result<(O::Bind, Observation)> {
    let bind = observer.bind(registration).await?;
    let observation = observer.next(&bind).await?;
    Ok((bind, observation))
}

async fn release<O: Observer>(observer: &O, binds: &[O::Bind]) {
    for bind in binds {
        let _ = observer.cancel(bind).await;
    }
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
    loop {
        if cancel.is_cancelled() {
            if let Some(binds) = bound.as_ref() {
                release(observer, binds).await;
            }
            return Ok(finish(
                set,
                request,
                clock.now(),
                outcome::cancelled(request, clock.now()),
            ));
        }
        if let Some(done) = terminal_deadline(set, request, &clock.now()) {
            if let Some(binds) = bound.as_ref() {
                release(observer, binds).await;
            }
            return Ok(finish(set, request, clock.now(), done));
        }

        let wait_until = earliest_deadline(request);
        if let Some(binds) = bound.as_ref() {
            tokio::select! {
                biased;
                ready = FirstReady::new(
                    binds.iter().map(|bind| observer.next(bind)),
                    observation_is_terminal,
                ) => {
                    let mut ready = ready?;
                    extend_ready(observer, binds, &mut ready);
                    if let Some(outcome) = decide(set, request, &clock.now(), &ready) {
                        release(observer, binds).await;
                        return Ok(finish(set, request, clock.now(), outcome));
                    }
                    backoff_step = backoff_step.saturating_add(1);
                    sleep_backoff(clock, cancel, request, backoff_step).await;
                    if cancel.is_cancelled() {
                        release(observer, binds).await;
                        return Ok(finish(
                            set,
                            request,
                            clock.now(),
                            outcome::cancelled(request, clock.now()),
                        ));
                    }
                }
                _ = cancel.cancelled() => {
                    release(observer, binds).await;
                    return Ok(finish(
                        set,
                        request,
                        clock.now(),
                        outcome::cancelled(request, clock.now()),
                    ));
                }
                _ = clock.sleep_until(wait_until) => {
                    let now = clock.now();
                    let mut ready = Vec::new();
                    extend_ready(observer, binds, &mut ready);
                    if let Some(outcome) = decide(set, request, &now, &ready) {
                        release(observer, binds).await;
                        return Ok(finish(set, request, now, outcome));
                    }
                    if let Some(done) = terminal_deadline(set, request, &now) {
                        release(observer, binds).await;
                        return Ok(finish(set, request, now, done));
                    }
                }
            }
            continue;
        }

        tokio::select! {
            biased;
            ready = FirstReady::new(
                set.registrations
                    .iter()
                    .map(|registration| bind_then_observe(observer, registration)),
                bound_observation_is_terminal,
            ) => {
                let ready = ready?;
                let mut indexed_binds = Vec::new();
                let mut observations = Vec::new();
                for (idx, (bind, observation)) in ready {
                    observations.push((idx, observation));
                    indexed_binds.push((idx, bind));
                }
                extend_ready_at(observer, &indexed_binds, &mut observations);
                if let Some(outcome) = decide(set, request, &clock.now(), &observations) {
                    let held: Vec<O::Bind> =
                        indexed_binds.into_iter().map(|(_, bind)| bind).collect();
                    release(observer, &held).await;
                    return Ok(finish(set, request, clock.now(), outcome));
                }
                indexed_binds.sort_by_key(|(idx, _)| *idx);
                let held: Vec<O::Bind> = indexed_binds.into_iter().map(|(_, bind)| bind).collect();
                bound = Some(held);
                backoff_step = backoff_step.saturating_add(1);
                sleep_backoff(clock, cancel, request, backoff_step).await;
            }
            _ = cancel.cancelled() => {
                return Ok(finish(
                    set,
                    request,
                    clock.now(),
                    outcome::cancelled(request, clock.now()),
                ));
            }
            _ = clock.sleep_until(wait_until) => {
                let now = clock.now();
                if let Some(done) = terminal_deadline(set, request, &now) {
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

fn extend_ready<O: Observer>(
    observer: &O,
    binds: &[O::Bind],
    ready: &mut Vec<(usize, Observation)>,
) {
    let indexed: Vec<(usize, &O::Bind)> = binds.iter().enumerate().collect();
    extend_ready_refs(observer, &indexed, ready);
}

fn extend_ready_at<O: Observer>(
    observer: &O,
    binds: &[(usize, O::Bind)],
    ready: &mut Vec<(usize, Observation)>,
) {
    let indexed: Vec<(usize, &O::Bind)> = binds.iter().map(|(idx, bind)| (*idx, bind)).collect();
    extend_ready_refs(observer, &indexed, ready);
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

fn decide(
    set: &RegistrationSet,
    request: &LiveWaitRequest,
    now: &Timestamp,
    ready: &[(usize, Observation)],
) -> Option<LiveWaitOutcome> {
    let mut events = Vec::new();
    let mut overflow = false;
    let mut failed = None;
    for (idx, obs) in ready {
        match obs {
            Observation::Event(event) => events.push((*idx, event.as_ref().clone())),
            Observation::Overflow => overflow = true,
            Observation::Failed { reason_code } => failed = Some(reason_code.as_str().to_string()),
            Observation::Idle => {}
        }
    }

    if overflow {
        let winner = events
            .into_iter()
            .min_by_key(|(idx, _)| *idx)
            .map(|(_, event)| event);
        return Some(if winner.is_some() {
            outcome::partial(set, request, now.clone(), winner)
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
) -> Option<LiveWaitOutcome> {
    if now >= &request.logical_deadline {
        return Some(outcome::logical_deadman(set, request, now.clone()));
    }
    if now >= &request.run_deadline {
        return Some(outcome::no_change(set, request, now.clone()));
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
    if set
        .registrations
        .iter()
        .any(|reg| now > reg.lease_expires_at)
    {
        return outcome::reauthentication_required(request, now, "lease_expired");
    }
    if set.authn_mode == AuthnMode::Required && request.verification_receipt_ref.is_none() {
        return outcome::refused(request, now, "authn_required");
    }
    outcome
}
