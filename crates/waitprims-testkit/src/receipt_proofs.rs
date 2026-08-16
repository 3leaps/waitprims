//! CP5 proofs: observe, wait result, and deliver/activate refs stay distinct.

use std::fs;
use std::path::PathBuf;

use waitprims_async::{run_first_match, run_poll_cycle, Cancel, Observation};
use waitprims_core::{
    attach_event_refs, validate_message, validate_raw_documents, AgentWaitMessage, MessageType,
    OpaqueRef, OutcomeKind,
};

use crate::{
    live_wait_request, poll_cycle_request, registration, registration_set, wait_event, FakeClock,
    Script, ScriptedObserver, ScriptedReceipts,
};

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/initial-case")
}

fn two_arm_set() -> waitprims_core::RegistrationSet {
    registration_set(vec![
        registration("reg:chanvoy-1", "chanvoy_wait", "chan:seat-a"),
        registration("reg:sms-1", "sms_inbound", "sms:inbox-1"),
    ])
}

fn sms_script() -> Script {
    Script {
        buffer_limit: 8,
        events: vec![wait_event(
            "reg:sms-1",
            "sms_inbound",
            "evt:sms-1",
            "2026-08-15T16:05:00Z",
        )],
    }
}

fn admit_live(outcome: &waitprims_core::LiveWaitOutcome) {
    let message = AgentWaitMessage::LiveWaitOutcome(outcome.clone());
    assert!(
        message.message_type().is_wait_result(),
        "live wait result must be a wait-result kind"
    );
    let json = serde_json::to_string(&message).expect("serialize live outcome");
    validate_message(&json).unwrap_or_else(|err| panic!("live outcome must admit: {err}; {json}"));
}

fn admit_poll(outcome: &waitprims_core::PollCycleOutcome) {
    let message = AgentWaitMessage::PollCycleOutcome(outcome.clone());
    assert!(
        message.message_type().is_wait_result(),
        "poll wait result must be a wait-result kind"
    );
    let json = serde_json::to_string(&message).expect("serialize poll outcome");
    validate_message(&json).unwrap_or_else(|err| panic!("poll outcome must admit: {err}; {json}"));
}

fn assert_one_wait_kind(kind: OutcomeKind) {
    assert!(
        OutcomeKind::ALL.contains(&kind),
        "wait result must use one of the nine outcome_kind values: {kind:?}"
    );
}

#[test]
fn observed_event_with_payload_ref_is_not_a_wait_outcome() {
    let event = wait_event(
        "reg:sms-1",
        "sms_inbound",
        "evt:sms-1",
        "2026-08-15T16:05:00Z",
    );
    assert_eq!(event.payload.payload_ref.as_str(), "msg:payload-1");
    let observation = Observation::Event(Box::new(event.clone()));
    assert!(
        !observation.is_wait_result(),
        "an observation is a candidate, not a wait result"
    );

    let event_json = serde_json::to_string(&event).expect("serialize candidate event");
    let err = validate_message(&event_json).expect_err("candidate event must not admit");
    assert!(
        err.to_string().contains("message_type") || err.to_string().contains("required"),
        "candidate event must fail as a wait message: {err}"
    );
    assert!(
        !event_json.contains("live_wait_outcome"),
        "candidate event must not claim a wait result: {event_json}"
    );
    assert!(
        !event_json.contains("poll_cycle_outcome"),
        "candidate event must not claim a wait result: {event_json}"
    );
    assert!(
        !event_json.contains("outcome_kind"),
        "candidate event must not carry outcome_kind: {event_json}"
    );
}

#[tokio::test]
async fn successful_wait_does_not_imply_delivery_or_activation() {
    let set = two_arm_set();
    let request = live_wait_request();
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = ScriptedObserver::new(sms_script(), clock.clone());
    let receipts = ScriptedReceipts::new();
    let outcome = run_first_match(&set, &request, &observer, &clock, &Cancel::new())
        .await
        .expect("first-match");
    admit_live(&outcome);
    assert_one_wait_kind(outcome.outcome_kind);
    assert_eq!(outcome.outcome_kind, OutcomeKind::Events);
    let events = outcome.events.as_ref().expect("events");
    assert_eq!(events.len(), 1);
    assert!(
        events[0].delivery_ref.is_none(),
        "match must not invent delivery"
    );
    assert!(
        events[0].activation_ref.is_none(),
        "match must not invent activation"
    );
    assert!(
        receipts.deliveries().is_empty() && receipts.activations().is_empty(),
        "a successful wait is not deliver/activate evidence"
    );

    let poll_request = poll_cycle_request(&set);
    let clock = FakeClock::auto(poll_request.created_at.clone());
    let observer = ScriptedObserver::new(sms_script(), clock.clone());
    let poll = run_poll_cycle(&set, &poll_request, &observer, &clock, &Cancel::new())
        .await
        .expect("poll cycle");
    admit_poll(&poll);
    assert_one_wait_kind(poll.outcome_kind);
    assert_eq!(poll.outcome_kind, OutcomeKind::Events);
    assert!(
        !poll.events.is_empty(),
        "poll events outcome must keep observed events"
    );
    for event in &poll.events {
        assert!(
            event.delivery_ref.is_none(),
            "poll match must not invent delivery"
        );
        assert!(
            event.activation_ref.is_none(),
            "request activation_ref must not be stamped onto observed events: {}",
            event.event_id.as_str()
        );
    }
    assert_ne!(
        poll_request.activation_ref.as_str(),
        "",
        "poll request still cites its own activation_ref"
    );
}

#[tokio::test]
async fn attaching_refs_does_not_change_kind_or_message_type() {
    let set = two_arm_set();
    let request = live_wait_request();
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = ScriptedObserver::new(sms_script(), clock.clone());
    let outcome = run_first_match(&set, &request, &observer, &clock, &Cancel::new())
        .await
        .expect("first-match");
    admit_live(&outcome);
    let kind = outcome.outcome_kind;
    let mut with_refs = outcome.clone();
    attach_event_refs(
        with_refs.events.as_mut().expect("events"),
        Some(OpaqueRef::new("del:caller-1")),
        Some(OpaqueRef::new("act:caller-1")),
    );
    assert_eq!(with_refs.outcome_kind, kind);
    assert_eq!(
        AgentWaitMessage::LiveWaitOutcome(with_refs.clone()).message_type(),
        MessageType::LiveWaitOutcome
    );
    admit_live(&with_refs);
    let tagged = with_refs.events.as_ref().expect("events");
    assert_eq!(
        tagged[0].delivery_ref.as_ref().map(OpaqueRef::as_str),
        Some("del:caller-1")
    );
    assert_eq!(
        tagged[0].activation_ref.as_ref().map(OpaqueRef::as_str),
        Some("act:caller-1")
    );

    let mut scripted = wait_event(
        "reg:sms-1",
        "sms_inbound",
        "evt:sms-1",
        "2026-08-15T16:05:00Z",
    );
    scripted.attach_refs(
        Some(OpaqueRef::new("del:script-1")),
        Some(OpaqueRef::new("act:script-1")),
    );
    let poll_request = poll_cycle_request(&set);
    let clock = FakeClock::auto(poll_request.created_at.clone());
    let observer = ScriptedObserver::new(
        Script {
            buffer_limit: 8,
            events: vec![scripted],
        },
        clock.clone(),
    );
    let poll = run_poll_cycle(&set, &poll_request, &observer, &clock, &Cancel::new())
        .await
        .expect("poll with caller refs");
    admit_poll(&poll);
    assert_eq!(poll.outcome_kind, OutcomeKind::Events);
    assert_eq!(
        AgentWaitMessage::PollCycleOutcome(poll.clone()).message_type(),
        MessageType::PollCycleOutcome
    );
    assert_eq!(
        poll.events[0].delivery_ref.as_ref().map(OpaqueRef::as_str),
        Some("del:script-1")
    );
    assert_eq!(
        poll.events[0]
            .activation_ref
            .as_ref()
            .map(OpaqueRef::as_str),
        Some("act:script-1")
    );
}

#[test]
fn scripted_deliver_activate_is_not_a_wire_kind() {
    let mut receipts = ScriptedReceipts::new();
    let delivery = receipts.deliver("del:scripted-1");
    let activation = receipts.activate("act:scripted-1");
    assert_eq!(receipts.deliveries().len(), 1);
    assert_eq!(receipts.activations().len(), 1);

    let delivery_json = serde_json::json!({
        "delivery_ref": delivery.delivery_ref().as_str()
    });
    assert!(delivery_json.get("message_type").is_none());
    validate_message(&delivery_json.to_string())
        .expect_err("delivery evidence must not admit as a wait message");

    let activation_json = serde_json::json!({
        "activation_ref": activation.activation_ref().as_str()
    });
    assert!(activation_json.get("message_type").is_none());
    validate_message(&activation_json.to_string())
        .expect_err("activation evidence must not admit as a wait message");

    let fake_delivery = serde_json::json!({
        "capabilities": ["contract: agent-wait/v0"],
        "message_type": "delivery",
        "delivery_ref": delivery.delivery_ref().as_str()
    });
    let err = validate_message(&fake_delivery.to_string()).expect_err("delivery kind");
    assert!(
        err.to_string().contains("undeclared_message_type"),
        "invented delivery kind must be undeclared: {err}"
    );

    let fake_activation = serde_json::json!({
        "capabilities": ["contract: agent-wait/v0"],
        "message_type": "activation",
        "activation_ref": activation.activation_ref().as_str()
    });
    let err = validate_message(&fake_activation.to_string()).expect_err("activation kind");
    assert!(
        err.to_string().contains("undeclared_message_type"),
        "invented activation kind must be undeclared: {err}"
    );

    let names: Vec<_> = MessageType::ALL.iter().map(|kind| kind.as_str()).collect();
    assert!(!names.contains(&"delivery"));
    assert!(!names.contains(&"activation"));
    assert!(!names.contains(&"live_wait_ack"));
}

#[tokio::test]
async fn initial_case_fixtures_still_admit_honest_outcomes() {
    let root = fixture_root();
    let set_raw = fs::read_to_string(root.join("registration_set.json")).expect("set fixture");
    let live_req_raw =
        fs::read_to_string(root.join("live_wait_request.json")).expect("live request");
    let poll_req_raw =
        fs::read_to_string(root.join("poll_cycle_request.json")).expect("poll request");
    let live_script =
        Script::from_json(&fs::read_to_string(root.join("live.json")).expect("live script"))
            .expect("parse live script");
    let poll_script =
        Script::from_json(&fs::read_to_string(root.join("poll.json")).expect("poll script"))
            .expect("parse poll script");

    validate_raw_documents([&set_raw, &live_req_raw]).expect("live pair must admit");
    validate_raw_documents([&set_raw, &poll_req_raw]).expect("poll pair must admit");

    let set = match validate_message(&set_raw)
        .expect("registration_set")
        .into_inner()
    {
        AgentWaitMessage::RegistrationSet(set) => set,
        other => panic!("expected registration_set, got {:?}", other.message_type()),
    };
    let live_request = match validate_message(&live_req_raw)
        .expect("live_wait_request")
        .into_inner()
    {
        AgentWaitMessage::LiveWaitRequest(request) => request,
        other => panic!("expected live_wait_request, got {:?}", other.message_type()),
    };
    let poll_request = match validate_message(&poll_req_raw)
        .expect("poll_cycle_request")
        .into_inner()
    {
        AgentWaitMessage::PollCycleRequest(request) => request,
        other => panic!(
            "expected poll_cycle_request, got {:?}",
            other.message_type()
        ),
    };

    let clock = FakeClock::auto(live_request.created_at.clone());
    let observer = ScriptedObserver::new(live_script, clock.clone());
    let live = run_first_match(&set, &live_request, &observer, &clock, &Cancel::new())
        .await
        .expect("initial-case live");
    admit_live(&live);
    assert_one_wait_kind(live.outcome_kind);
    assert_eq!(live.outcome_kind, OutcomeKind::Events);
    let live_events = live.events.as_ref().expect("live events");
    assert_eq!(live_events[0].registration_id.as_str(), "reg:sms-1");
    assert!(live_events[0].delivery_ref.is_none());
    assert!(live_events[0].activation_ref.is_none());
    assert!(live.arms.is_none());
    assert!(live.coverage_complete.is_none());

    let clock = FakeClock::auto(poll_request.created_at.clone());
    let observer = ScriptedObserver::new(poll_script, clock.clone());
    let poll = run_poll_cycle(&set, &poll_request, &observer, &clock, &Cancel::new())
        .await
        .expect("initial-case poll");
    admit_poll(&poll);
    assert_one_wait_kind(poll.outcome_kind);
    assert_eq!(poll.outcome_kind, OutcomeKind::Events);
    assert!(poll.coverage_complete);
    assert_eq!(poll.arms.len(), 3);
    for event in &poll.events {
        assert!(
            event.delivery_ref.is_none(),
            "initial-case poll must not invent delivery"
        );
        assert!(
            event.activation_ref.is_none(),
            "initial-case poll must not stamp request activation onto match"
        );
    }
}
