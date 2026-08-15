//! Builders for sterile first-match cases.

use waitprims_core::{
    ActorRef, Anchor, AnchorKind, AuthnMode, Canonicalization, CapabilityToken, ContentDigest,
    DigestAlgorithm, IdToken, JcsDigest, LiveWaitRequest, OpaqueRef, PayloadRef, PredicateRef,
    Registration, RegistrationSet, ReplayStatus, Timestamp, WaitBound, WaitEvent,
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

/// Scripted source event. `delivery_ref` / `activation_ref` stay unset.
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
