//! Live first-match runner.
//!
//! # Tie rule
//!
//! When two or more arms accept an observation at the same logical instant,
//! the winner is the arm that appears first in
//! `registration_set.registrations`. That order is the only tie break.

use std::time::Duration;

use waitprims_core::{
    AuthnMode, LiveWaitOutcome, LiveWaitRequest, OutcomeKind, RegistrationSet, Result, Timestamp,
    ValidationError,
};

use crate::cancel::Cancel;
use crate::clock::Clock;
use crate::observer::{Observation, Observer};
use crate::outcome;
use crate::race::ObservationRace;

/// Documented deterministic tie rule for same-instant accepted observations.
pub const TIE_RULE: &str =
    "same-instant winner is the earliest arm in registration_set.registrations";

const BACKOFF_MS: &[u64] = &[50, 100, 200, 400, 800, 1000];

/// Resolve the cited registration set/revision and first-match an observer.
///
/// Emits a `live_wait_outcome` body. Callers serialize
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

    let mut binds = Vec::with_capacity(set.registrations.len());
    for registration in &set.registrations {
        binds.push(observer.bind(registration).await?);
    }

    let outcome = run_loop(set, request, observer, clock, cancel, &binds).await;
    for bind in &binds {
        let _ = observer.cancel(bind).await;
    }
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

async fn run_loop<O, C>(
    set: &RegistrationSet,
    request: &LiveWaitRequest,
    observer: &O,
    clock: &C,
    cancel: &Cancel,
    binds: &[O::Bind],
) -> Result<LiveWaitOutcome>
where
    O: Observer,
    C: Clock,
{
    let mut backoff_step = 0usize;
    loop {
        if cancel.is_cancelled() {
            return Ok(finish(
                set,
                request,
                clock.now(),
                outcome::cancelled(request, clock.now()),
            ));
        }
        if let Some(done) = terminal_deadline(set, request, &clock.now()) {
            return Ok(finish(set, request, clock.now(), done));
        }

        tokio::select! {
            biased;
            ready = ObservationRace::new(binds.iter().map(|bind| observer.next(bind))) => {
                let mut ready = ready?;
                extend_ready(observer, binds, &mut ready);
                if let Some(outcome) = decide(set, request, &clock.now(), &ready) {
                    return Ok(finish(set, request, clock.now(), outcome));
                }
                backoff_step = backoff_step.saturating_add(1);
                let sleep_to = backoff_deadline(clock, request, backoff_step);
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => {
                        return Ok(finish(
                            set,
                            request,
                            clock.now(),
                            outcome::cancelled(request, clock.now()),
                        ));
                    }
                    _ = clock.sleep_until(&sleep_to) => {}
                }
            }
            _ = cancel.cancelled() => {
                return Ok(finish(
                    set,
                    request,
                    clock.now(),
                    outcome::cancelled(request, clock.now()),
                ));
            }
            _ = clock.sleep_until(&request.logical_deadline) => {
                let now = clock.now();
                let mut ready = Vec::new();
                extend_ready(observer, binds, &mut ready);
                if let Some(outcome) = decide(set, request, &now, &ready) {
                    return Ok(finish(set, request, now, outcome));
                }
                if let Some(done) = terminal_deadline(set, request, &now) {
                    return Ok(finish(set, request, now, done));
                }
            }
        }
    }
}

fn extend_ready<O: Observer>(
    observer: &O,
    binds: &[O::Bind],
    ready: &mut Vec<(usize, Observation)>,
) {
    for (idx, bind) in binds.iter().enumerate() {
        if ready.iter().any(|(have, _)| *have == idx) {
            continue;
        }
        if let Some(obs) = observer.poll_ready(bind) {
            ready.push((idx, obs));
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
    Some(outcome::events(set, request, now.clone(), event))
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

fn backoff_deadline<C: Clock>(clock: &C, request: &LiveWaitRequest, step: usize) -> Timestamp {
    let idx = step.saturating_sub(1).min(BACKOFF_MS.len() - 1);
    let delay = Duration::from_millis(BACKOFF_MS[idx]);
    let wake = clock.now().saturating_add(delay);
    let cap = if request.run_deadline < request.logical_deadline {
        request.run_deadline.clone()
    } else {
        request.logical_deadline.clone()
    };
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
