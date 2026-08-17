//! Proofs: first-match, deadline, cancel, drop, ties, restore, overflow, starvation.

use waitprims_async::{run_first_match, Cancel, Observation, Observer, TIE_RULE};
use waitprims_core::{
    validate_message, AgentWaitMessage, AuthnMode, LiveWaitOutcome, OpaqueRef, OutcomeKind,
    Registration, Result, Timestamp, ValidationError,
};

use crate::{
    exclusive_head_anchor, live_wait_request, registration, registration_baseline,
    registration_set, ts, wait_event, FakeClock, IdleObserver, Script, ScriptedObserver,
    TrackedBind,
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

fn three_arm_baseline_set() -> waitprims_core::RegistrationSet {
    registration_set(vec![
        registration_baseline("reg:chanvoy-1", "chanvoy_wait", "chan:seat-a"),
        registration_baseline("reg:sms-1", "sms_inbound", "sms:inbox-1"),
        registration_baseline("reg:job-1", "job_complete", "job:transcribe-1"),
    ])
}

fn assert_clean_coverage(outcome: &LiveWaitOutcome, expected: &[(&str, &str)]) {
    let arms = outcome.arms.as_ref().expect("coverage arms");
    assert_eq!(arms.len(), expected.len(), "must keep every bound arm");
    for (arm, (reg, start)) in arms.iter().zip(expected) {
        assert_eq!(arm.registration_id.as_str(), *reg);
        assert_eq!(arm.start_anchor.value.as_str(), *start);
        assert_ne!(arm.start_anchor.value.as_str(), "anc:baseline-latest");
    }
    let json = serde_json::to_string(outcome).expect("serialize");
    assert!(
        !json.contains("anc:baseline-latest"),
        "must not mint a policy label as a cursor: {json}"
    );
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
async fn same_instant_loser_is_restored_for_next_wait() {
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
    let first = run_first_match(&set, &request, &observer, &clock, &Cancel::new())
        .await
        .expect("first-match");
    admit(&first);
    assert_eq!(first.outcome_kind, OutcomeKind::Events);
    assert_eq!(
        first.events.as_ref().unwrap()[0].registration_id.as_str(),
        "reg:chanvoy-1"
    );
    assert_eq!(
        first.events.as_ref().unwrap()[0].event_id.as_str(),
        "evt:chanvoy-1"
    );
    let queued = observer.queued_event_ids();
    assert_eq!(
        queued.get("reg:sms-1").map(Vec::as_slice),
        Some(["evt:sms-1".to_string()].as_slice()),
        "same-instant loser must be restored: {queued:?}"
    );
    assert!(
        queued.get("reg:chanvoy-1").is_none_or(|ids| ids.is_empty()),
        "winner must stay consumed: {queued:?}"
    );
    assert_eq!(observer.live_bind_count(), 0);

    let second = run_first_match(&set, &request, &observer, &clock, &Cancel::new())
        .await
        .expect("replay wait");
    admit(&second);
    assert_eq!(second.outcome_kind, OutcomeKind::Events);
    assert_eq!(
        second.events.as_ref().unwrap()[0].event_id.as_str(),
        "evt:sms-1"
    );
    assert_eq!(
        second.events.as_ref().unwrap()[0].registration_id.as_str(),
        "reg:sms-1"
    );
    assert_eq!(observer.live_bind_count(), 0);
}

#[tokio::test]
async fn same_instant_winner_is_registration_order_not_backfill_or_wall() {
    assert_eq!(
        TIE_RULE,
        "same-instant winner is the earliest arm in registration_set.registrations"
    );
    let set = two_arm_set();
    let request = live_wait_request();
    let observed = "2026-08-15T16:05:00Z";
    let mut sms = wait_event("reg:sms-1", "sms_inbound", "evt:aaa-sms", observed);
    sms.occurred_at = ts("2026-08-15T16:00:00Z");
    let mut chanvoy = wait_event("reg:chanvoy-1", "chanvoy_wait", "evt:zzz-chanvoy", observed);
    chanvoy.occurred_at = ts("2026-08-15T16:09:00Z");
    let script = Script {
        buffer_limit: 8,
        events: vec![sms, chanvoy],
    };
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = ScriptedObserver::new(script, clock.clone());
    let outcome = run_first_match(&set, &request, &observer, &clock, &Cancel::new())
        .await
        .expect("tie");
    admit(&outcome);
    assert_eq!(outcome.outcome_kind, OutcomeKind::Events);
    let events = outcome.events.expect("events");
    assert_eq!(events[0].registration_id.as_str(), "reg:chanvoy-1");
    assert_eq!(events[0].event_id.as_str(), "evt:zzz-chanvoy");
    assert_eq!(events[0].observed_at, ts(observed));
    assert_eq!(events[0].occurred_at, ts("2026-08-15T16:09:00Z"));
    assert_eq!(observer.live_bind_count(), 0);

    let reversed = registration_set(vec![
        registration("reg:sms-1", "sms_inbound", "sms:inbox-1"),
        registration("reg:chanvoy-1", "chanvoy_wait", "chan:seat-a"),
    ]);
    let mut sms = wait_event("reg:sms-1", "sms_inbound", "evt:zzz-sms", observed);
    sms.occurred_at = ts("2026-08-15T16:09:00Z");
    let mut chanvoy = wait_event("reg:chanvoy-1", "chanvoy_wait", "evt:aaa-chanvoy", observed);
    chanvoy.occurred_at = ts("2026-08-15T16:00:00Z");
    let script = Script {
        buffer_limit: 8,
        events: vec![chanvoy, sms],
    };
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = ScriptedObserver::new(script, clock.clone());
    let outcome = run_first_match(&reversed, &request, &observer, &clock, &Cancel::new())
        .await
        .expect("reversed tie");
    admit(&outcome);
    assert_eq!(
        outcome.events.as_ref().unwrap()[0].registration_id.as_str(),
        "reg:sms-1",
        "winner must follow the reversed registration set, not wall or script order"
    );
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
    assert_clean_coverage(
        &outcome,
        &[
            ("reg:chanvoy-1", "anc:cursor-0"),
            ("reg:sms-1", "anc:cursor-0"),
            ("reg:job-1", "anc:cursor-0"),
        ],
    );
}

#[tokio::test]
async fn baseline_policy_empty_wait_is_admitted_no_change() {
    let set = three_arm_baseline_set();
    let request = live_wait_request();
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = ScriptedObserver::new(Script::default(), clock.clone());
    let outcome = run_first_match(&set, &request, &observer, &clock, &Cancel::new())
        .await
        .expect("no_change");
    admit(&outcome);
    assert_eq!(outcome.outcome_kind, OutcomeKind::NoChange);
    assert_eq!(outcome.completed_at, ts("2026-08-15T16:20:00Z"));
    assert_eq!(observer.live_bind_count(), 0);
    assert_eq!(
        exclusive_head_anchor(&set.registrations[0].registration_id)
            .value
            .as_str(),
        "anc:h-chanvoy-1"
    );
    assert_clean_coverage(
        &outcome,
        &[
            ("reg:chanvoy-1", "anc:h-chanvoy-1"),
            ("reg:sms-1", "anc:h-sms-1"),
            ("reg:job-1", "anc:h-job-1"),
        ],
    );
}

#[tokio::test]
async fn baseline_policy_idle_wait_is_admitted_logical_deadman() {
    let set = three_arm_baseline_set();
    let mut request = live_wait_request();
    request.logical_deadline = ts("2026-08-15T16:02:00Z");
    request.run_deadline = request.logical_deadline.clone();
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = IdleObserver::new();
    let outcome = run_first_match(&set, &request, &observer, &clock, &Cancel::new())
        .await
        .expect("deadman");
    admit(&outcome);
    assert_eq!(outcome.outcome_kind, OutcomeKind::LogicalDeadman);
    assert_eq!(observer.live_bind_count(), 0);
    assert_clean_coverage(
        &outcome,
        &[
            ("reg:chanvoy-1", "anc:h-chanvoy-1"),
            ("reg:sms-1", "anc:h-sms-1"),
            ("reg:job-1", "anc:h-job-1"),
        ],
    );
}

async fn hanging_required_bind_at_deadline(
    set: waitprims_core::RegistrationSet,
    hang: &[&str],
    logical: &str,
    run: &str,
) {
    let mut request = live_wait_request();
    request.logical_deadline = ts(logical);
    request.run_deadline = ts(run);
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = ScriptedObserver::new(Script::default(), clock.clone());
    for registration_id in hang {
        observer.hang_bind(registration_id);
    }
    let outcome = run_first_match(&set, &request, &observer, &clock, &Cancel::new())
        .await
        .expect("terminal outcome");
    admit(&outcome);
    assert_ne!(
        outcome.outcome_kind,
        OutcomeKind::NoChange,
        "incomplete required bind must not be clean no_change"
    );
    assert_ne!(
        outcome.outcome_kind,
        OutcomeKind::LogicalDeadman,
        "incomplete required bind must not be clean logical_deadman"
    );
    assert_ne!(
        outcome.coverage_complete,
        Some(true),
        "incomplete required bind must not claim coverage_complete"
    );
    assert_eq!(outcome.outcome_kind, OutcomeKind::Failed);
    assert_eq!(
        outcome.reason_code.as_ref().map(|code| code.as_str()),
        Some("required_bind_pending")
    );
    assert_eq!(observer.live_bind_count(), 0);
}

#[tokio::test]
async fn hanging_required_bind_at_run_deadline_is_failed_not_clean() {
    let run = "2026-08-15T16:20:00Z";
    let logical = "2026-08-15T17:00:00Z";
    for set in [three_arm_set(), three_arm_baseline_set()] {
        hanging_required_bind_at_deadline(set.clone(), &["reg:chanvoy-1"], logical, run).await;
        hanging_required_bind_at_deadline(
            set,
            &["reg:chanvoy-1", "reg:sms-1", "reg:job-1"],
            logical,
            run,
        )
        .await;
    }
}

#[tokio::test]
async fn hanging_required_bind_at_logical_deadline_is_failed_not_clean() {
    let at = "2026-08-15T16:02:00Z";
    for set in [three_arm_set(), three_arm_baseline_set()] {
        hanging_required_bind_at_deadline(set.clone(), &["reg:chanvoy-1"], at, at).await;
        hanging_required_bind_at_deadline(
            set,
            &["reg:chanvoy-1", "reg:sms-1", "reg:job-1"],
            at,
            at,
        )
        .await;
    }
}

#[tokio::test]
async fn hanging_cancel_does_not_delay_decided_outcome() {
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
    observer.hang_cancel();
    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        run_first_match(&set, &request, &observer, &clock, &Cancel::new()),
    )
    .await
    .expect("hung cancel must not delay a decided outcome")
    .expect("runner");
    admit(&outcome);
    assert_eq!(outcome.outcome_kind, OutcomeKind::Events);
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

/// Dequeues from `next` only. Default `poll_ready` does not consume.
struct NextOnlyObserver {
    inner: ScriptedObserver,
}

impl Observer for NextOnlyObserver {
    type Bind = TrackedBind;

    async fn bind(&self, registration: &Registration) -> Result<Self::Bind> {
        self.inner.bind(registration).await
    }

    async fn next(&self, bind: &Self::Bind) -> Result<Observation> {
        self.inner.next(bind).await
    }

    async fn cancel(&self, bind: &Self::Bind) -> Result<()> {
        self.inner.cancel(bind).await
    }

    fn restore_ready(&self, bind: &Self::Bind, obs: Observation) -> Result<()> {
        self.inner.restore_ready(bind, obs)
    }
}

/// One arm returns `next` error while a sibling can emit an event.
struct EventPlusError {
    inner: ScriptedObserver,
    fail: String,
}

impl Observer for EventPlusError {
    type Bind = TrackedBind;

    async fn bind(&self, registration: &Registration) -> Result<Self::Bind> {
        self.inner.bind(registration).await
    }

    async fn next(&self, bind: &Self::Bind) -> Result<Observation> {
        if bind.registration_id.as_str() == self.fail {
            return Err(ValidationError::new("/observer", "injected_arm_error").into());
        }
        self.inner.next(bind).await
    }

    async fn cancel(&self, bind: &Self::Bind) -> Result<()> {
        self.inner.cancel(bind).await
    }

    fn poll_ready(&self, bind: &Self::Bind) -> Option<Observation> {
        if bind.registration_id.as_str() == self.fail {
            return None;
        }
        self.inner.poll_ready(bind)
    }

    fn restore_ready(&self, bind: &Self::Bind, obs: Observation) -> Result<()> {
        self.inner.restore_ready(bind, obs)
    }
}

#[tokio::test]
async fn first_ready_error_restores_same_instant_event_for_next_wait() {
    let set = two_arm_set();
    let request = live_wait_request();
    let at = "2026-08-15T16:05:00Z";
    let script = Script {
        buffer_limit: 8,
        events: vec![
            wait_event("reg:chanvoy-1", "chanvoy_wait", "evt:chanvoy-1", at),
            wait_event("reg:sms-1", "sms_inbound", "evt:sms-1", at),
        ],
    };
    let clock = FakeClock::auto(request.created_at.clone());
    let inner = ScriptedObserver::new(script, clock.clone());
    let observer = EventPlusError {
        inner: inner.clone(),
        fail: "reg:sms-1".to_string(),
    };
    let err = run_first_match(&set, &request, &observer, &clock, &Cancel::new())
        .await
        .expect_err("sibling error");
    assert!(
        err.to_string().contains("injected_arm_error"),
        "unexpected error: {err}"
    );
    assert_eq!(
        inner
            .queued_event_ids()
            .get("reg:chanvoy-1")
            .map(Vec::as_slice),
        Some(["evt:chanvoy-1".to_string()].as_slice()),
        "same-instant event collected before the error must be restored: {:?}",
        inner.queued_event_ids()
    );
    assert_eq!(
        inner.queued_event_ids().get("reg:sms-1").map(Vec::as_slice),
        Some(["evt:sms-1".to_string()].as_slice())
    );
    assert_eq!(inner.live_bind_count(), 0);

    let replay = run_first_match(&set, &request, &inner, &clock, &Cancel::new())
        .await
        .expect("replay");
    admit(&replay);
    assert_eq!(replay.outcome_kind, OutcomeKind::Events);
    assert_eq!(
        replay.events.as_ref().unwrap()[0].event_id.as_str(),
        "evt:chanvoy-1"
    );
}

#[tokio::test]
async fn posture_reject_restores_selected_event_for_later_wait() {
    let mut set = registration_set(vec![registration(
        "reg:sms-1",
        "sms_inbound",
        "sms:inbox-1",
    )]);
    set.authn_mode = AuthnMode::Required;
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
    let refused = run_first_match(&set, &request, &observer, &clock, &Cancel::new())
        .await
        .expect("refused");
    admit(&refused);
    assert_eq!(refused.outcome_kind, OutcomeKind::Refused);
    assert_eq!(
        refused.reason_code.as_ref().map(|code| code.as_str()),
        Some("authn_required")
    );
    assert_eq!(
        observer
            .queued_event_ids()
            .get("reg:sms-1")
            .map(Vec::as_slice),
        Some(["evt:sms-1".to_string()].as_slice()),
        "auth-refused selected event must be restored: {:?}",
        observer.queued_event_ids()
    );

    let mut allowed = request.clone();
    allowed.verification_receipt_ref = Some(OpaqueRef::new("vr:seat-a"));
    let replay = run_first_match(&set, &allowed, &observer, &clock, &Cancel::new())
        .await
        .expect("replay after receipt");
    admit(&replay);
    assert_eq!(replay.outcome_kind, OutcomeKind::Events);
    assert_eq!(
        replay.events.as_ref().unwrap()[0].event_id.as_str(),
        "evt:sms-1"
    );

    let mut leased = registration("reg:sms-1", "sms_inbound", "sms:inbox-1");
    leased.lease_expires_at = ts("2026-08-15T16:04:00Z");
    let lease_set = registration_set(vec![leased]);
    let lease_script = Script {
        buffer_limit: 8,
        events: vec![wait_event(
            "reg:sms-1",
            "sms_inbound",
            "evt:lease-1",
            "2026-08-15T16:05:00Z",
        )],
    };
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = ScriptedObserver::new(lease_script, clock.clone());
    let reauth = run_first_match(&lease_set, &request, &observer, &clock, &Cancel::new())
        .await
        .expect("reauth");
    admit(&reauth);
    assert_eq!(reauth.outcome_kind, OutcomeKind::ReauthenticationRequired);
    assert_eq!(
        observer
            .queued_event_ids()
            .get("reg:sms-1")
            .map(Vec::as_slice),
        Some(["evt:lease-1".to_string()].as_slice()),
        "lease-expired selected event must be restored: {:?}",
        observer.queued_event_ids()
    );
    let mut later = registration("reg:sms-1", "sms_inbound", "sms:inbox-1");
    later.lease_expires_at = ts("2026-08-16T00:00:00Z");
    let later_set = registration_set(vec![later]);
    let replay = run_first_match(&later_set, &request, &observer, &clock, &Cancel::new())
        .await
        .expect("replay after lease");
    admit(&replay);
    assert_eq!(replay.outcome_kind, OutcomeKind::Events);
    assert_eq!(
        replay.events.as_ref().unwrap()[0].event_id.as_str(),
        "evt:lease-1"
    );
}

#[tokio::test]
async fn next_only_same_instant_loser_is_restored() {
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
    let inner = ScriptedObserver::new(script, clock.clone());
    let observer = NextOnlyObserver {
        inner: inner.clone(),
    };
    let first = run_first_match(&set, &request, &observer, &clock, &Cancel::new())
        .await
        .expect("next-only first-match");
    admit(&first);
    assert_eq!(first.outcome_kind, OutcomeKind::Events);
    assert_eq!(
        first.events.as_ref().unwrap()[0].event_id.as_str(),
        "evt:chanvoy-1"
    );
    assert_eq!(
        inner.queued_event_ids().get("reg:sms-1").map(Vec::as_slice),
        Some(["evt:sms-1".to_string()].as_slice()),
        "next()-sourced loser must restore even when poll_ready never dequeues: {:?}",
        inner.queued_event_ids()
    );
    let second = run_first_match(&set, &request, &observer, &clock, &Cancel::new())
        .await
        .expect("next-only replay");
    admit(&second);
    assert_eq!(
        second.events.as_ref().unwrap()[0].event_id.as_str(),
        "evt:sms-1"
    );
}
