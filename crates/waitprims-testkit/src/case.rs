//! Builders for sterile first-match cases.

use std::collections::BTreeMap;

use waitprims_core::{
    ActorRef, Anchor, AnchorKind, AuthnMode, BaselinePolicy, Canonicalization, CapabilityToken,
    ContentDigest, DigestAlgorithm, IdToken, JcsDigest, LiveWaitRequest, OpaqueRef, PayloadRef,
    PollCycleAck, PollCycleOutcome, PollCycleRequest, PredicateRef, Registration, RegistrationSet,
    ReplayStatus, Timestamp, WaitBound, WaitEvent,
};

/// Parse a fixture timestamp.
pub fn ts(raw: &str) -> Timestamp {
    Timestamp::parse(raw).expect("fixture timestamp")
}

/// Registration set used by proofs and the initial CLI case.
pub fn registration_set(registrations: Vec<Registration>) -> RegistrationSet {
    RegistrationSet {
        capabilities: vec![CapabilityToken::new("contract: agent-wait/v0")],
        message_id: IdToken::new("msg:aw-reg-1"),
        correlation_id: IdToken::new("corr:aw-1"),
        created_at: ts("2026-08-15T16:00:00Z"),
        actor_ref: ActorRef::new("seat:consumer-a"),
        causation_id: None,
        grant_ref: None,
        verification_receipt_ref: None,
        policy_decision_ref: None,
        principal_ref: ActorRef::new("seat:consumer-a"),
        waiter_id: IdToken::new("waiter:seat-consumer-a"),
        seat_ref: OpaqueRef::new("seat:consumer-a"),
        registration_revision: IdToken::new("regrev-1"),
        logical_deadline: ts("2026-08-15T17:00:00Z"),
        authn_mode: AuthnMode::Optional,
        aggregate_limits: WaitBound {
            max_events: 100,
            max_bytes: 1_048_576,
        },
        registration_digest: JcsDigest {
            canonicalization: Canonicalization::Rfc8785,
            algorithm: DigestAlgorithm::Sha256,
            value: "0".repeat(64),
        },
        registrations,
    }
}

/// One required registration with a declared exclusive start cursor.
pub fn registration(registration_id: &str, method_id: &str, subject_id: &str) -> Registration {
    Registration {
        registration_id: IdToken::new(registration_id),
        method_id: IdToken::new(method_id),
        subject_kind: IdToken::new("subject"),
        subject_id: IdToken::new(subject_id),
        required: true,
        source_instance_ref: OpaqueRef::new("source:provider-a"),
        predicate_ref: PredicateRef::new("pred:match"),
        capability_ref: OpaqueRef::new("cap:wait"),
        lease_expires_at: ts("2026-08-16T00:00:00Z"),
        bounds: WaitBound {
            max_events: 50,
            max_bytes: 524_288,
        },
        start_anchor: Some(Anchor {
            kind: AnchorKind::ProviderOpaque,
            value: IdToken::new("anc:cursor-0"),
        }),
        baseline_policy: None,
    }
}

/// One required registration that starts from `baseline_policy=latest`.
pub fn registration_baseline(
    registration_id: &str,
    method_id: &str,
    subject_id: &str,
) -> Registration {
    Registration {
        registration_id: IdToken::new(registration_id),
        method_id: IdToken::new(method_id),
        subject_kind: IdToken::new("subject"),
        subject_id: IdToken::new(subject_id),
        required: true,
        source_instance_ref: OpaqueRef::new("source:provider-a"),
        predicate_ref: PredicateRef::new("pred:match"),
        capability_ref: OpaqueRef::new("cap:wait"),
        lease_expires_at: ts("2026-08-16T00:00:00Z"),
        bounds: WaitBound {
            max_events: 50,
            max_bytes: 524_288,
        },
        start_anchor: None,
        baseline_policy: Some(BaselinePolicy::Latest),
    }
}

/// Arm id assigned to a registration in poll-cycle fixtures.
pub fn arm_id_for(registration: &Registration) -> IdToken {
    let rest = registration
        .registration_id
        .as_str()
        .strip_prefix("reg:")
        .unwrap_or(registration.registration_id.as_str());
    IdToken::new(format!("arm:{rest}"))
}

/// Poll-cycle request citing [`registration_set`].
pub fn poll_cycle_request(set: &RegistrationSet) -> PollCycleRequest {
    let required_arms = set
        .registrations
        .iter()
        .filter(|registration| registration.required)
        .map(arm_id_for)
        .collect();
    PollCycleRequest {
        capabilities: vec![CapabilityToken::new("contract: agent-wait/v0")],
        message_id: IdToken::new("msg:aw-poll-req-1"),
        correlation_id: IdToken::new("corr:aw-1"),
        created_at: ts("2026-08-15T16:01:00Z"),
        actor_ref: ActorRef::new("seat:consumer-a"),
        causation_id: Some(IdToken::new("msg:aw-reg-1")),
        grant_ref: None,
        verification_receipt_ref: None,
        policy_decision_ref: None,
        waiter_id: IdToken::new("waiter:seat-consumer-a"),
        registration_set_ref: IdToken::new("msg:aw-reg-1"),
        registration_revision: IdToken::new("regrev-1"),
        logical_deadline: ts("2026-08-15T17:00:00Z"),
        run_deadline: ts("2026-08-15T16:20:00Z"),
        required_arms,
        fairness_cursor: IdToken::new("fair:start"),
        acknowledged_anchors: BTreeMap::new(),
        activation_ref: OpaqueRef::new("act:cycle-1"),
        cycle_id: IdToken::new("cycle:1"),
        bound: None,
    }
}

/// Commit the retained cursors and event ids from an admitted poll outcome.
pub fn ack_poll_outcome(outcome: &PollCycleOutcome) -> PollCycleAck {
    PollCycleAck {
        capabilities: outcome.capabilities.clone(),
        message_id: IdToken::new(format!("{}:ack", outcome.message_id.as_str())),
        correlation_id: outcome.correlation_id.clone(),
        created_at: outcome.completed_at.clone(),
        actor_ref: outcome.actor_ref.clone(),
        causation_id: Some(outcome.message_id.clone()),
        grant_ref: outcome.grant_ref.clone(),
        verification_receipt_ref: outcome.verification_receipt_ref.clone(),
        policy_decision_ref: outcome.policy_decision_ref.clone(),
        waiter_id: outcome.waiter_id.clone(),
        outcome_ref: outcome.message_id.clone(),
        committed_anchors: outcome.retained_through.clone(),
        retained_events: outcome.retained_events.clone(),
    }
}

/// Live wait request citing [`registration_set`].
pub fn live_wait_request() -> LiveWaitRequest {
    LiveWaitRequest {
        capabilities: vec![CapabilityToken::new("contract: agent-wait/v0")],
        message_id: IdToken::new("msg:aw-live-req-1"),
        correlation_id: IdToken::new("corr:aw-1"),
        created_at: ts("2026-08-15T16:01:00Z"),
        actor_ref: ActorRef::new("seat:consumer-a"),
        causation_id: Some(IdToken::new("msg:aw-reg-1")),
        grant_ref: None,
        verification_receipt_ref: None,
        policy_decision_ref: None,
        waiter_id: IdToken::new("waiter:seat-consumer-a"),
        registration_set_ref: IdToken::new("msg:aw-reg-1"),
        registration_revision: IdToken::new("regrev-1"),
        logical_deadline: ts("2026-08-15T17:00:00Z"),
        run_deadline: ts("2026-08-15T16:20:00Z"),
    }
}

/// Scripted source event. `delivery_ref` / `activation_ref` stay unset so
/// a match cannot be mistaken for deliver/activate evidence.
pub fn wait_event(registration_id: &str, method_id: &str, event_id: &str, at: &str) -> WaitEvent {
    WaitEvent {
        event_id: IdToken::new(event_id),
        registration_id: IdToken::new(registration_id),
        source_instance_ref: OpaqueRef::new("source:provider-a"),
        method_id: IdToken::new(method_id),
        subject_kind: IdToken::new("subject"),
        subject_id: IdToken::new("subject:1"),
        occurred_at: ts(at),
        observed_at: ts(at),
        start_anchor: Anchor {
            kind: AnchorKind::ProviderOpaque,
            value: IdToken::new("anc:cursor-0"),
        },
        proposed_next_anchor: Anchor {
            kind: AnchorKind::ProviderOpaque,
            value: IdToken::new("anc:after-1"),
        },
        replay_status: ReplayStatus::Fresh,
        correlation_id: IdToken::new("corr:aw-1"),
        causation_id: None,
        payload: PayloadRef {
            payload_ref: OpaqueRef::new("msg:payload-1"),
            content_digest: ContentDigest {
                algorithm: DigestAlgorithm::Sha256,
                value: "c".repeat(64),
            },
            media_type: None,
        },
        delivery_ref: None,
        activation_ref: None,
    }
}
