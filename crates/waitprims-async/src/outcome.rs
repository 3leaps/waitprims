//! Build admitted `live_wait_outcome` values from a first-match decision.

use waitprims_core::{
    Anchor, AnchorKind, ArmStatus, CoverageArm, IdToken, LiveWaitOutcome, LiveWaitRequest,
    OutcomeKind, Registration, RegistrationSet, Timestamp, WaitEvent,
};

pub(crate) fn start_anchor(registration: &Registration) -> Anchor {
    if let Some(anchor) = &registration.start_anchor {
        return anchor.clone();
    }
    let value = match registration.baseline_policy {
        Some(waitprims_core::BaselinePolicy::Latest) => "anc:baseline-latest",
        Some(waitprims_core::BaselinePolicy::Earliest) => "anc:baseline-earliest",
        Some(waitprims_core::BaselinePolicy::ProviderDefined) => "anc:baseline-provider",
        None => "anc:baseline",
    };
    Anchor {
        kind: AnchorKind::ProviderOpaque,
        value: IdToken::new(value),
    }
}

fn arm_id(registration: &Registration) -> IdToken {
    IdToken::new(format!("arm:{}", registration.registration_id.as_str()))
}

fn coverage_arm(
    registration: &Registration,
    status: ArmStatus,
    event: Option<&WaitEvent>,
) -> CoverageArm {
    let start = start_anchor(registration);
    let proposed = event
        .map(|ev| ev.proposed_next_anchor.clone())
        .unwrap_or_else(|| start.clone());
    CoverageArm {
        arm_id: arm_id(registration),
        registration_id: registration.registration_id.clone(),
        required: registration.required,
        status,
        degraded: false,
        start_anchor: event.map(|ev| ev.start_anchor.clone()).unwrap_or(start),
        proposed_next_anchor: proposed,
        event_count: u64::from(event.is_some()),
        byte_count: 0,
        reason_code: None,
    }
}

fn base(request: &LiveWaitRequest, completed_at: Timestamp, kind: OutcomeKind) -> LiveWaitOutcome {
    LiveWaitOutcome {
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
        outcome_kind: kind,
        logical_deadline: None,
        events: None,
        proposed_next_anchor: None,
        coverage_complete: None,
        arms: None,
        reason_code: None,
    }
}

fn arms_for(
    set: &RegistrationSet,
    winner: Option<&WaitEvent>,
    winner_status: ArmStatus,
    loser_status: ArmStatus,
) -> Vec<CoverageArm> {
    set.registrations
        .iter()
        .map(|reg| {
            let is_winner = winner
                .map(|ev| ev.registration_id.as_str() == reg.registration_id.as_str())
                .unwrap_or(false);
            if is_winner {
                coverage_arm(reg, winner_status, winner)
            } else {
                coverage_arm(reg, loser_status, None)
            }
        })
        .collect()
}

pub(crate) fn events(
    set: &RegistrationSet,
    request: &LiveWaitRequest,
    completed_at: Timestamp,
    event: WaitEvent,
) -> LiveWaitOutcome {
    let proposed = event.proposed_next_anchor.clone();
    let arms = arms_for(set, Some(&event), ArmStatus::Events, ArmStatus::NoChange);
    let mut outcome = base(request, completed_at, OutcomeKind::Events);
    outcome.events = Some(vec![event]);
    outcome.proposed_next_anchor = Some(proposed);
    outcome.coverage_complete = Some(true);
    outcome.arms = Some(arms);
    outcome
}

pub(crate) fn partial(
    set: &RegistrationSet,
    request: &LiveWaitRequest,
    completed_at: Timestamp,
    event: Option<WaitEvent>,
) -> LiveWaitOutcome {
    let proposed = event.as_ref().map(|ev| ev.proposed_next_anchor.clone());
    let arms = arms_for(set, event.as_ref(), ArmStatus::Events, ArmStatus::Deferred);
    let mut outcome = base(request, completed_at, OutcomeKind::Partial);
    outcome.events = Some(event.into_iter().collect());
    outcome.proposed_next_anchor = proposed;
    outcome.coverage_complete = Some(false);
    outcome.arms = Some(arms);
    outcome
}

pub(crate) fn logical_deadman(
    set: &RegistrationSet,
    request: &LiveWaitRequest,
    completed_at: Timestamp,
) -> LiveWaitOutcome {
    let arms = arms_for(set, None, ArmStatus::NoChange, ArmStatus::NoChange);
    let mut outcome = base(request, completed_at, OutcomeKind::LogicalDeadman);
    outcome.events = Some(Vec::new());
    outcome.logical_deadline = Some(request.logical_deadline.clone());
    outcome.coverage_complete = Some(true);
    outcome.arms = Some(arms);
    outcome
}

pub(crate) fn no_change(
    set: &RegistrationSet,
    request: &LiveWaitRequest,
    completed_at: Timestamp,
) -> LiveWaitOutcome {
    let arms = arms_for(set, None, ArmStatus::NoChange, ArmStatus::NoChange);
    let mut outcome = base(request, completed_at, OutcomeKind::NoChange);
    outcome.events = Some(Vec::new());
    outcome.logical_deadline = Some(request.logical_deadline.clone());
    outcome.coverage_complete = Some(true);
    outcome.arms = Some(arms);
    outcome
}

pub(crate) fn cancelled(request: &LiveWaitRequest, completed_at: Timestamp) -> LiveWaitOutcome {
    let mut outcome = base(request, completed_at, OutcomeKind::Cancelled);
    outcome.reason_code = Some(IdToken::new("consumer_cancelled"));
    outcome.events = Some(Vec::new());
    outcome
}

pub(crate) fn failed(
    request: &LiveWaitRequest,
    completed_at: Timestamp,
    reason_code: &str,
) -> LiveWaitOutcome {
    let mut outcome = base(request, completed_at, OutcomeKind::Failed);
    outcome.reason_code = Some(IdToken::new(reason_code));
    outcome.events = Some(Vec::new());
    outcome
}

pub(crate) fn refused(
    request: &LiveWaitRequest,
    completed_at: Timestamp,
    reason_code: &str,
) -> LiveWaitOutcome {
    let mut outcome = base(request, completed_at, OutcomeKind::Refused);
    outcome.reason_code = Some(IdToken::new(reason_code));
    outcome.events = Some(Vec::new());
    outcome
}

pub(crate) fn reauthentication_required(
    request: &LiveWaitRequest,
    completed_at: Timestamp,
    reason_code: &str,
) -> LiveWaitOutcome {
    let mut outcome = base(request, completed_at, OutcomeKind::ReauthenticationRequired);
    outcome.reason_code = Some(IdToken::new(reason_code));
    outcome.events = Some(Vec::new());
    outcome
}
