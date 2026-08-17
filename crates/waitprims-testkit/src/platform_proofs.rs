//! Platform proofs: logical clock, restore/ack, and portable cancel.
//!
//! [`FakeClock`] is logical. Same-instant first-match, restore, and poll-ack
//! use contract timestamps and registration-set order. Do not key uniqueness
//! on a wall `sleep`; Windows timer granularity is coarser than Unix 1ms.
//! Cancel and deadline use the portable watch token and
//! [`waitprims_async::Clock`], not `EINTR`, UDS, or signals.

use waitprims_async::{run_first_match, run_poll_cycle, Cancel, POLL_ACK_RETENTION, TIE_RULE};
use waitprims_core::{
    validate_message, AgentWaitMessage, IdToken, LiveWaitOutcome, OutcomeKind, PollBound,
    PollCycleOutcome,
};

use crate::{
    ack_poll_outcome, live_wait_request, poll_cycle_request, registration, registration_set, ts,
    wait_event, FakeClock, IdleObserver, Script, ScriptedObserver,
};

fn admit_live(outcome: &LiveWaitOutcome) {
    let message = AgentWaitMessage::LiveWaitOutcome(outcome.clone());
    let json = serde_json::to_string(&message).expect("serialize outcome");
    validate_message(&json).unwrap_or_else(|err| panic!("outcome must admit: {err}; {json}"));
}

fn admit_poll(outcome: &PollCycleOutcome) {
    let message = AgentWaitMessage::PollCycleOutcome(outcome.clone());
    let json = serde_json::to_string(&message).expect("serialize outcome");
    validate_message(&json).unwrap_or_else(|err| panic!("outcome must admit: {err}; {json}"));
}

fn two_arm_set() -> waitprims_core::RegistrationSet {
    registration_set(vec![
        registration("reg:chanvoy-1", "chanvoy_wait", "chan:seat-a"),
        registration("reg:sms-1", "sms_inbound", "sms:inbox-1"),
    ])
}

#[tokio::test]
async fn same_instant_tie_uses_registration_order_not_wall_clock() {
    assert_eq!(
        TIE_RULE,
        "same-instant winner is the earliest arm in registration_set.registrations"
    );
    let set = two_arm_set();
    let request = live_wait_request();
    let at = "2026-08-15T16:05:00Z";
    let sms = wait_event("reg:sms-1", "sms_inbound", "evt:sms-1", at);
    let chanvoy = wait_event("reg:chanvoy-1", "chanvoy_wait", "evt:chanvoy-1", at);
    assert_eq!(
        sms.observed_at, chanvoy.observed_at,
        "same observed_at is the tie key, not a wall Instant"
    );
    let script = Script {
        buffer_limit: 8,
        events: vec![sms, chanvoy],
    };
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = ScriptedObserver::new(script, clock.clone());
    let outcome = run_first_match(&set, &request, &observer, &clock, &Cancel::new())
        .await
        .expect("tie");
    admit_live(&outcome);
    assert_eq!(outcome.outcome_kind, OutcomeKind::Events);
    let events = outcome.events.expect("events");
    assert_eq!(events[0].registration_id.as_str(), "reg:chanvoy-1");
    assert_eq!(events[0].event_id.as_str(), "evt:chanvoy-1");
    assert_eq!(
        observer
            .queued_event_ids()
            .get("reg:sms-1")
            .map(Vec::as_slice),
        Some(["evt:sms-1".to_string()].as_slice()),
        "same-instant loser must restore by event_id: {:?}",
        observer.queued_event_ids()
    );
}

#[tokio::test]
async fn coarse_logical_clock_harvest_uses_registration_order_and_restores() {
    let set = two_arm_set();
    let request = live_wait_request();
    let earlier = wait_event(
        "reg:sms-1",
        "sms_inbound",
        "evt:sms-1",
        "2026-08-15T16:05:00.001000Z",
    );
    let later = wait_event(
        "reg:chanvoy-1",
        "chanvoy_wait",
        "evt:chanvoy-1",
        "2026-08-15T16:05:00.002000Z",
    );
    assert_ne!(
        earlier.observed_at, later.observed_at,
        "contract timestamps stay distinct; FakeClock does not collapse them"
    );
    // Logical now is already past both instants, as a coarse OS timer would
    // report if it cannot split 1ms. The harvest is registration-order and
    // the loser must still restore.
    let clock = FakeClock::auto(ts("2026-08-15T16:05:00.016000Z"));
    let script = Script {
        buffer_limit: 8,
        events: vec![later, earlier],
    };
    let observer = ScriptedObserver::new(script, clock.clone());
    let first = run_first_match(&set, &request, &observer, &clock, &Cancel::new())
        .await
        .expect("coarse harvest");
    admit_live(&first);
    assert_eq!(first.outcome_kind, OutcomeKind::Events);
    assert_eq!(
        first.events.as_ref().unwrap()[0].event_id.as_str(),
        "evt:chanvoy-1",
        "when the logical clock is past both instants, winner is registration order"
    );
    assert_eq!(
        observer
            .queued_event_ids()
            .get("reg:sms-1")
            .map(Vec::as_slice),
        Some(["evt:sms-1".to_string()].as_slice()),
        "loser must restore: {:?}",
        observer.queued_event_ids()
    );
    let second = run_first_match(&set, &request, &observer, &clock, &Cancel::new())
        .await
        .expect("restored loser");
    admit_live(&second);
    assert_eq!(
        second.events.as_ref().unwrap()[0].event_id.as_str(),
        "evt:sms-1"
    );
}

#[tokio::test]
async fn restore_identity_is_event_id_not_wall_instant() {
    let set = two_arm_set();
    let request = live_wait_request();
    let at = "2026-08-15T16:05:00.000001Z";
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
    admit_live(&first);
    assert_eq!(
        first.events.as_ref().unwrap()[0].registration_id.as_str(),
        "reg:chanvoy-1"
    );
    let queued = observer.queued_event_ids();
    assert_eq!(
        queued.get("reg:sms-1").map(Vec::as_slice),
        Some(["evt:sms-1".to_string()].as_slice()),
        "restore must keep the loser event_id: {queued:?}"
    );
    let second = run_first_match(&set, &request, &observer, &clock, &Cancel::new())
        .await
        .expect("replay");
    admit_live(&second);
    assert_eq!(
        second.events.as_ref().unwrap()[0].event_id.as_str(),
        "evt:sms-1"
    );
}

#[tokio::test]
async fn poll_ack_commit_is_not_a_clock_tick() {
    assert!(
        POLL_ACK_RETENTION.contains("not committed until poll_cycle_ack"),
        "ack retention must stay fail-closed: {POLL_ACK_RETENTION}"
    );
    let set = registration_set(vec![registration(
        "reg:sms-1",
        "sms_inbound",
        "sms:inbox-1",
    )]);
    let request = poll_cycle_request(&set);
    let script = Script {
        buffer_limit: 8,
        events: vec![wait_event(
            "reg:sms-1",
            "sms_inbound",
            "evt:sms-1",
            "2026-08-15T16:05:00.001000Z",
        )],
    };
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = ScriptedObserver::new(script, clock.clone());
    let first = run_poll_cycle(&set, &request, &observer, &clock, &Cancel::new())
        .await
        .expect("poll");
    admit_poll(&first);
    assert_eq!(first.events[0].event_id.as_str(), "evt:sms-1");
    let start = first
        .arms
        .iter()
        .find(|arm| arm.registration_id.as_str() == "reg:sms-1")
        .map(|arm| arm.start_anchor.clone())
        .expect("start");
    let mut second_req = request.clone();
    second_req.message_id = IdToken::new("msg:aw-poll-req-2");
    let second = run_poll_cycle(&set, &second_req, &observer, &clock, &Cancel::new())
        .await
        .expect("unacked replay");
    admit_poll(&second);
    assert_eq!(
        second.events[0].event_id.as_str(),
        "evt:sms-1",
        "a later FakeClock read is not poll_cycle_ack"
    );
    assert_eq!(
        second
            .arms
            .iter()
            .find(|arm| arm.registration_id.as_str() == "reg:sms-1")
            .map(|arm| &arm.start_anchor),
        Some(&start),
        "unacked restart must keep the original start cursor"
    );

    let ack = ack_poll_outcome(&first);
    let mut committed_req = request.clone();
    committed_req.message_id = IdToken::new("msg:aw-poll-req-3");
    committed_req.acknowledged_anchors = ack.committed_anchors.clone();
    let clock = FakeClock::auto(committed_req.created_at.clone());
    let committed = run_poll_cycle(
        &set,
        &committed_req,
        &ScriptedObserver::new(Script::default(), clock.clone()),
        &clock,
        &Cancel::new(),
    )
    .await
    .expect("acked cycle");
    admit_poll(&committed);
    assert!(committed.events.is_empty());
    assert_eq!(
        committed
            .arms
            .iter()
            .find(|arm| arm.registration_id.as_str() == "reg:sms-1")
            .map(|arm| &arm.start_anchor),
        first.proposed_next_anchors.get("reg:sms-1")
    );
}

#[tokio::test]
async fn poll_same_instant_defer_restores_by_event_id() {
    let set = two_arm_set();
    let at = "2026-08-15T16:05:00.000001Z";
    let script = Script {
        buffer_limit: 8,
        events: vec![
            wait_event("reg:chanvoy-1", "chanvoy_wait", "evt:left", at),
            wait_event("reg:sms-1", "sms_inbound", "evt:right", at),
        ],
    };
    let mut first = poll_cycle_request(&set);
    first.bound = Some(PollBound {
        max_events: Some(1),
        max_payload_refs: None,
        max_bytes: None,
    });
    let clock = FakeClock::auto(first.created_at.clone());
    let observer = ScriptedObserver::new(script, clock.clone());
    let left = run_poll_cycle(&set, &first, &observer, &clock, &Cancel::new())
        .await
        .expect("first cycle");
    admit_poll(&left);
    assert_eq!(left.events[0].event_id.as_str(), "evt:left");
    let mut second = first.clone();
    second.message_id = IdToken::new("msg:aw-poll-req-2");
    second.fairness_cursor = left.next_fairness_cursor.clone();
    let right = run_poll_cycle(&set, &second, &observer, &clock, &Cancel::new())
        .await
        .expect("rotated cycle");
    admit_poll(&right);
    assert_eq!(
        right.events[0].event_id.as_str(),
        "evt:right",
        "deferred same-instant event must restore by event_id: {:?}",
        right.events
    );
}

async fn prove_portable_deadline_and_cancel() {
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
    let outcome = run_first_match(&set, &request, &observer, &clock, &Cancel::new())
        .await
        .expect("logical deadline");
    admit_live(&outcome);
    assert_eq!(outcome.outcome_kind, OutcomeKind::LogicalDeadman);
    assert_eq!(observer.live_bind_count(), 0);

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
    let cancelled = task.await.expect("join").expect("cancelled");
    admit_live(&cancelled);
    assert_eq!(cancelled.outcome_kind, OutcomeKind::Cancelled);
    assert_eq!(observer.live_bind_count(), 0);
}

#[tokio::test]
async fn portable_deadline_and_cancel_do_not_need_eintr() {
    prove_portable_deadline_and_cancel().await;
}

#[cfg(windows)]
#[tokio::test]
async fn windows_deadline_and_cancel_use_portable_clock_and_watch_token() {
    prove_portable_deadline_and_cancel().await;
}
