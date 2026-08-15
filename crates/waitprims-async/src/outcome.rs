//! Build admitted `live_wait_outcome` values from a first-match decision.

use waitprims_core::{
    ArmStatus, CoverageArm, IdToken, LiveWaitOutcome, LiveWaitRequest, OutcomeKind, Registration,
    RegistrationSet, Timestamp, WaitEvent,
};

fn arm_id(registration: &Registration) -> IdToken {
    IdToken::new(format!("arm:{}", registration.registration_id.as_str()))
}

fn coverage_arm(
    registration: &Registration,
    status: ArmStatus,
    event: Option<&WaitEvent>,
) -> Option<CoverageArm> {
    let start = event
        .map(|ev| ev.start_anchor.clone())
        .or_else(|| registration.start_anchor.clone())?;
    let proposed = event
        .map(|ev| ev.proposed_next_anchor.clone())
        .unwrap_or_else(|| start.clone());
    Some(CoverageArm {
        arm_id: arm_id(registration),
        registration_id: registration.registration_id.clone(),
        required: registration.required,
        status,
        degraded: false,
        start_anchor: start,
        proposed_next_anchor: proposed,
        event_count: u64::from(event.is_some()),
        byte_count: 0,
        reason_code: None,
    })
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

fn clean_arms(set: &RegistrationSet) -> Vec<CoverageArm> {
    set.registrations
        .iter()
        .filter_map(|reg| coverage_arm(reg, ArmStatus::NoChange, None))
        .collect()
}

pub(crate) fn events(
    request: &LiveWaitRequest,
    completed_at: Timestamp,
    event: WaitEvent,
) -> LiveWaitOutcome {
    let proposed = event.proposed_next_anchor.clone();
    let mut outcome = base(request, completed_at, OutcomeKind::Events);
    outcome.events = Some(vec![event]);
    outcome.proposed_next_anchor = Some(proposed);
    outcome
}

pub(crate) fn partial(
    set: &RegistrationSet,
    request: &LiveWaitRequest,
    completed_at: Timestamp,
    event: Option<WaitEvent>,
) -> LiveWaitOutcome {
    let proposed = event.as_ref().map(|ev| ev.proposed_next_anchor.clone());
    let winner_id = event
        .as_ref()
        .map(|ev| ev.registration_id.as_str().to_string());
    let arms: Vec<CoverageArm> = set
        .registrations
        .iter()
        .filter_map(|reg| {
            let is_winner = winner_id
                .as_deref()
                .is_some_and(|id| id == reg.registration_id.as_str());
            if is_winner {
                coverage_arm(reg, ArmStatus::Events, event.as_ref())
            } else {
                coverage_arm(reg, ArmStatus::Deferred, None)
            }
        })
        .collect();
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
    let mut outcome = base(request, completed_at, OutcomeKind::LogicalDeadman);
    outcome.events = Some(Vec::new());
    outcome.logical_deadline = Some(request.logical_deadline.clone());
    outcome.coverage_complete = Some(true);
    outcome.arms = Some(clean_arms(set));
    outcome
}

pub(crate) fn no_change(
    set: &RegistrationSet,
    request: &LiveWaitRequest,
    completed_at: Timestamp,
) -> LiveWaitOutcome {
    let mut outcome = base(request, completed_at, OutcomeKind::NoChange);
    outcome.events = Some(Vec::new());
    outcome.logical_deadline = Some(request.logical_deadline.clone());
    outcome.coverage_complete = Some(true);
    outcome.arms = Some(clean_arms(set));
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
