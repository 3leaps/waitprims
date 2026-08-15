//! CP3 proofs: first-match, deadline, cancel, drop, ties, overflow, starvation.

use waitprims_async::{run_first_match, Cancel, TIE_RULE};
use waitprims_core::{validate_message, AgentWaitMessage, LiveWaitOutcome, OutcomeKind, Timestamp};

use crate::{
    live_wait_request, registration, registration_set, ts, wait_event, FakeClock, IdleObserver,
    Script, ScriptedObserver,
};

fn admit(outcome: &LiveWaitOutcome) {
    let message = AgentWaitMessage::LiveWaitOutcome(outcome.clone());
    let json = serde_json::to_string(&message).expect("serialize outcome");
    validate_message(&json).unwrap_or_else(|err| panic!("outcome must admit: {err}; {json}"));
}

#[test]
fn initial_case_registration_set_digest_matches() {
    let raw = include_str!("../../../fixtures/initial-case/registration_set.json");
    let value: serde_json::Value = serde_json::from_str(raw).expect("fixture json");
    let regs = serde_json::to_string(value.get("registrations").expect("registrations")).unwrap();
    let got = waitprims_core::registration_digest(&regs).expect("digest");
    assert_eq!(
        got,
        value["registration_digest"]["value"]
            .as_str()
            .expect("digest value"),
        "update fixtures/initial-case/registration_set.json digest to {got}"
    );
}

fn two_arm_set() -> waitprims_core::RegistrationSet {
    registration_set(vec![
        registration("reg:chanvoy-1", "chanvoy_wait", "chan:seat-a"),
        registration("reg:sms-1", "sms_inbound", "sms:inbox-1"),
    ])
}

fn three_arm_set() -> waitprims_core::RegistrationSet {
    registration_set(vec![
        registration("reg:chanvoy-1", "chanvoy_wait", "chan:seat-a"),
        registration("reg:sms-1", "sms_inbound", "sms:inbox-1"),
        registration("reg:job-1", "job_complete", "job:transcribe-1"),
    ])
}

#[tokio::test]
async fn two_arms_first_accepted_events_loser_cleaned() {
    let set = two_arm_set();
    let request = live_wait_request();
    let script = Script {
        buffer_limit: 8,
        events: vec![
            wait_event(
                "reg:chanvoy-1",
                "chanvoy_wait",
                "evt:chanvoy-1",
                "2026-08-15T16:10:00Z",
            ),
            wait_event(
                "reg:sms-1",
                "sms_inbound",
                "evt:sms-1",
                "2026-08-15T16:05:00Z",
            ),
        ],
    };
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = ScriptedObserver::new(script, clock.clone());
    let cancel = Cancel::new();
    let outcome = run_first_match(&set, &request, &observer, &clock, &cancel)
        .await
        .expect("first-match");
    admit(&outcome);
    assert_eq!(outcome.outcome_kind, OutcomeKind::Events);
    let events = outcome.events.as_ref().expect("events");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].registration_id.as_str(), "reg:sms-1");
    assert!(observer.cancelled_ids().contains("reg:chanvoy-1"));
    assert_eq!(observer.live_bind_count(), 0);
    assert!(
        outcome.arms.is_none(),
        "first-match events must not invent loser arms"
    );
    assert!(
        outcome.coverage_complete.is_none(),
        "first-match events must not claim complete coverage"
    );
    let json = serde_json::to_string(&outcome).expect("serialize");
    assert!(
        !json.contains("anc:baseline-latest"),
        "must not fabricate a policy cursor: {json}"
    );
}

#[tokio::test]
async fn logical_deadline_is_absolute_backoff_does_not_extend() {
    let set = registration_set(vec![registration(
        "reg:sms-1",
        "sms_inbound",
        "sms:inbox-1",
    )]);
    let mut request = live_wait_request();
    request.logical_deadline = ts("2026-08-15T16:02:00Z");
    request.run_deadline = request.logical_deadline.clone();
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = IdleObserver::new();
    let cancel = Cancel::new();
    let outcome = run_first_match(&set, &request, &observer, &clock, &cancel)
        .await
        .expect("deadman");
    admit(&outcome);
    assert_eq!(outcome.outcome_kind, OutcomeKind::LogicalDeadman);
    assert_eq!(
        outcome.completed_at,
        Timestamp::parse("2026-08-15T16:02:00Z").unwrap()
    );
    assert!(clock.current_time() <= request.logical_deadline);
    assert_eq!(observer.live_bind_count(), 0);
}

#[tokio::test]
async fn cancel_during_backoff_cancelled_no_leaked_registration() {
    let set = registration_set(vec![registration(
        "reg:sms-1",
        "sms_inbound",
        "sms:inbox-1",
    )]);
    let request = live_wait_request();
    let clock = FakeClock::manual(request.created_at.clone());
    let observer = IdleObserver::new();
    let cancel = Cancel::new();
    let set2 = set.clone();
    let request2 = request.clone();
    let clock2 = clock.clone();
    let observer2 = observer.clone();
    let cancel2 = cancel.clone();
    let task = tokio::spawn(async move {
        run_first_match(&set2, &request2, &observer2, &clock2, &cancel2).await
    });
    while clock.sleeper_count() == 0 {
        tokio::task::yield_now().await;
    }
    cancel.trigger();
    let outcome = task.await.expect("join").expect("cancelled");
    admit(&outcome);
    assert_eq!(outcome.outcome_kind, OutcomeKind::Cancelled);
    assert_eq!(observer.live_bind_count(), 0);
    assert!(observer.cancelled_ids().contains("reg:sms-1"));
}

#[tokio::test]
async fn drop_releases_binds() {
    let set = two_arm_set();
    let request = live_wait_request();
    let clock = FakeClock::manual(request.created_at.clone());
    let observer = ScriptedObserver::new(Script::default(), clock.clone());
    let cancel = Cancel::new();
    let set2 = set.clone();
    let request2 = request.clone();
    let clock2 = clock.clone();
    let observer2 = observer.clone();
    let cancel2 = cancel.clone();
    let task = tokio::spawn(async move {
        run_first_match(&set2, &request2, &observer2, &clock2, &cancel2).await
    });
    while observer.live_bind_count() == 0 {
        tokio::task::yield_now().await;
    }
    assert_eq!(observer.live_bind_count(), 2);
    task.abort();
    let _ = task.await;
    assert_eq!(observer.live_bind_count(), 0);
}

#[tokio::test]
async fn deterministic_tie_uses_registration_set_order() {
    assert!(
        TIE_RULE.contains("registration_set.registrations"),
        "tie rule must be documented: {TIE_RULE}"
    );
    let set = two_arm_set();
    let request = live_wait_request();
    let at = "2026-08-15T16:05:00Z";
    let script = Script {
        buffer_limit: 8,
        events: vec![
            wait_event("reg:sms-1", "sms_inbound", "evt:sms-1", at),
            wait_event("reg:chanvoy-1", "chanvoy_wait", "evt:chanvoy-1", at),
        ],
    };
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = ScriptedObserver::new(script, clock.clone());
    let cancel = Cancel::new();
    let outcome = run_first_match(&set, &request, &observer, &clock, &cancel)
        .await
        .expect("tie");
    admit(&outcome);
    assert_eq!(outcome.outcome_kind, OutcomeKind::Events);
    let events = outcome.events.expect("events");
    assert_eq!(events[0].registration_id.as_str(), "reg:chanvoy-1");
    assert_eq!(observer.live_bind_count(), 0);
}

#[tokio::test]
async fn bounded_buffer_overflow_is_failed_or_partial() {
    let set = two_arm_set();
    let request = live_wait_request();
    let at = "2026-08-15T16:05:00Z";

    let failed_script = Script {
        buffer_limit: 1,
        events: vec![
            wait_event("reg:sms-1", "sms_inbound", "evt:sms-1", at),
            wait_event("reg:sms-1", "sms_inbound", "evt:sms-2", at),
        ],
    };
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = ScriptedObserver::new(failed_script, clock.clone());
    let outcome = run_first_match(&set, &request, &observer, &clock, &Cancel::new())
        .await
        .expect("overflow failed");
    admit(&outcome);
    assert_eq!(outcome.outcome_kind, OutcomeKind::Failed);
    assert_eq!(
        outcome.reason_code.as_ref().map(|r| r.as_str()),
        Some("buffer_overflow")
    );
    assert_eq!(observer.live_bind_count(), 0);

    let partial_script = Script {
        buffer_limit: 1,
        events: vec![
            wait_event("reg:chanvoy-1", "chanvoy_wait", "evt:chanvoy-1", at),
            wait_event("reg:sms-1", "sms_inbound", "evt:sms-1", at),
            wait_event("reg:sms-1", "sms_inbound", "evt:sms-2", at),
        ],
    };
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = ScriptedObserver::new(partial_script, clock.clone());
    let outcome = run_first_match(&set, &request, &observer, &clock, &Cancel::new())
        .await
        .expect("overflow partial");
    admit(&outcome);
    assert_eq!(outcome.outcome_kind, OutcomeKind::Partial);
    assert_eq!(observer.live_bind_count(), 0);
}

#[tokio::test]
async fn slow_arm_cannot_starve_cancel_or_ready_sibling() {
    let set = three_arm_set();
    let request = live_wait_request();
    let script = Script {
        buffer_limit: 8,
        events: vec![wait_event(
            "reg:sms-1",
            "sms_inbound",
            "evt:sms-1",
            "2026-08-15T16:05:00Z",
        )],
    };
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = ScriptedObserver::new(script, clock.clone());
    let outcome = run_first_match(&set, &request, &observer, &clock, &Cancel::new())
        .await
        .expect("ready sibling");
    admit(&outcome);
    assert_eq!(outcome.outcome_kind, OutcomeKind::Events);
    assert_eq!(
        outcome.events.as_ref().unwrap()[0].registration_id.as_str(),
        "reg:sms-1"
    );
    assert!(observer.cancelled_ids().contains("reg:chanvoy-1"));
    assert!(observer.cancelled_ids().contains("reg:job-1"));
    assert_eq!(observer.live_bind_count(), 0);

    let clock = FakeClock::manual(request.created_at.clone());
    let observer = ScriptedObserver::new(Script::default(), clock.clone());
    let cancel = Cancel::new();
    let set2 = set.clone();
    let request2 = request.clone();
    let clock2 = clock.clone();
    let observer2 = observer.clone();
    let cancel2 = cancel.clone();
    let task = tokio::spawn(async move {
        run_first_match(&set2, &request2, &observer2, &clock2, &cancel2).await
    });
    while clock.sleeper_count() == 0 {
        tokio::task::yield_now().await;
    }
    cancel.trigger();
    let outcome = task.await.expect("join").expect("cancel vs hang");
    admit(&outcome);
    assert_eq!(outcome.outcome_kind, OutcomeKind::Cancelled);
    assert_eq!(observer.live_bind_count(), 0);
}

#[tokio::test]
async fn empty_pending_observer_honors_run_deadline_as_no_change() {
    let set = three_arm_set();
    let request = live_wait_request();
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = ScriptedObserver::new(Script::default(), clock.clone());
    let outcome = run_first_match(&set, &request, &observer, &clock, &Cancel::new())
        .await
        .expect("no_change");
    admit(&outcome);
    assert_eq!(outcome.outcome_kind, OutcomeKind::NoChange);
    assert_eq!(outcome.completed_at, ts("2026-08-15T16:20:00Z"));
    assert_eq!(clock.current_time(), ts("2026-08-15T16:20:00Z"));
    assert_eq!(
        outcome
            .logical_deadline
            .as_ref()
            .map(Timestamp::as_str)
            .map(str::to_string),
        Some("2026-08-15T17:00:00Z".to_string())
    );
    assert_eq!(observer.live_bind_count(), 0);
}

#[tokio::test]
async fn slow_bind_cannot_starve_cancel_or_ready_sibling() {
    let set = two_arm_set();
    let request = live_wait_request();
    let script = Script {
        buffer_limit: 8,
        events: vec![wait_event(
            "reg:sms-1",
            "sms_inbound",
            "evt:sms-1",
            "2026-08-15T16:05:00Z",
        )],
    };
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = ScriptedObserver::new(script, clock.clone());
    observer.hang_bind("reg:chanvoy-1");
    let outcome = run_first_match(&set, &request, &observer, &clock, &Cancel::new())
        .await
        .expect("ready sibling despite hanging bind");
    admit(&outcome);
    assert_eq!(outcome.outcome_kind, OutcomeKind::Events);
    assert_eq!(
        outcome.events.as_ref().unwrap()[0].registration_id.as_str(),
        "reg:sms-1"
    );
    assert!(outcome.arms.is_none());
    assert!(outcome.coverage_complete.is_none());
    assert_eq!(observer.live_bind_count(), 0);

    let clock = FakeClock::manual(request.created_at.clone());
    let observer = ScriptedObserver::new(Script::default(), clock.clone());
    observer.hang_bind("reg:chanvoy-1");
    observer.hang_bind("reg:sms-1");
    let cancel = Cancel::new();
    let set2 = set.clone();
    let request2 = request.clone();
    let clock2 = clock.clone();
    let observer2 = observer.clone();
    let cancel2 = cancel.clone();
    let task = tokio::spawn(async move {
        run_first_match(&set2, &request2, &observer2, &clock2, &cancel2).await
    });
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
    cancel.trigger();
    let outcome = task.await.expect("join").expect("cancel vs hanging bind");
    admit(&outcome);
    assert_eq!(outcome.outcome_kind, OutcomeKind::Cancelled);
    assert_eq!(observer.live_bind_count(), 0);
}
