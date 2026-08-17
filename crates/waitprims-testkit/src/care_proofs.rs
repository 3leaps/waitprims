//! Care-pack proofs: portable cancel/deadline, restore_ready fail-closed,
//! poll-ack same-observer, and no Job Object claim.
//!
//! [`crate::FakeClock`] stays logical. These tests do not use wall sleep
//! as a uniqueness key.

use waitprims_async::{
    run_first_match, run_poll_cycle, BindHandle, Cancel, Observation, Observer, POLL_ACK_RETENTION,
};
use waitprims_core::{
    validate_message, AgentWaitMessage, IdToken, LiveWaitOutcome, OutcomeKind, PollCycleOutcome,
    Registration, Result, ValidationError,
};

use crate::{
    ack_poll_outcome, live_wait_request, poll_cycle_request, registration, registration_set, ts,
    wait_event, FakeClock, IdleObserver, Script, ScriptedObserver, TrackedBind,
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

/// Observer that dequeues like [`ScriptedObserver`] but refuses restore.
struct RestoreFailObserver {
    inner: ScriptedObserver,
}

impl Observer for RestoreFailObserver {
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

    fn poll_ready(&self, bind: &Self::Bind) -> Option<Observation> {
        self.inner.poll_ready(bind)
    }

    fn restore_ready(&self, bind: &Self::Bind, _obs: Observation) -> Result<()> {
        Err(ValidationError::new(
            "/observer/restore_ready",
            format!("restore_failed:{}", bind.registration_id().as_str()),
        )
        .into())
    }
}

#[tokio::test]
async fn restore_ready_error_fail_closes_first_match() {
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
    let observer = RestoreFailObserver {
        inner: inner.clone(),
    };
    let err = run_first_match(&set, &request, &observer, &clock, &Cancel::new())
        .await
        .expect_err("restore_ready Err must fail closed");
    assert!(
        err.to_string().contains("restore_failed"),
        "unexpected restore error: {err}"
    );
    assert_eq!(inner.live_bind_count(), 0);
}

#[tokio::test]
async fn restore_ready_error_fail_closes_poll_cycle() {
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
            "2026-08-15T16:05:00Z",
        )],
    };
    let clock = FakeClock::auto(request.created_at.clone());
    let inner = ScriptedObserver::new(script, clock.clone());
    let observer = RestoreFailObserver {
        inner: inner.clone(),
    };
    let err = run_poll_cycle(&set, &request, &observer, &clock, &Cancel::new())
        .await
        .expect_err("restore_ready Err must not emit an admitted poll outcome");
    assert!(
        err.to_string().contains("restore_failed"),
        "unexpected restore error: {err}"
    );
    assert_eq!(inner.live_bind_count(), 0);
}

#[tokio::test]
async fn portable_cancel_does_not_advance_fake_clock() {
    let set = registration_set(vec![registration(
        "reg:sms-1",
        "sms_inbound",
        "sms:inbox-1",
    )]);
    let request = live_wait_request();
    let start = request.created_at.clone();
    let clock = FakeClock::manual(start.clone());
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
    assert_eq!(clock.current_time(), start);
    cancel.trigger();
    let outcome = task.await.expect("join").expect("cancelled");
    admit_live(&outcome);
    assert_eq!(outcome.outcome_kind, OutcomeKind::Cancelled);
    assert_eq!(
        clock.current_time(),
        start,
        "Cancel is a watch token; it must not tick FakeClock"
    );
    assert_eq!(observer.live_bind_count(), 0);
}

#[tokio::test]
async fn portable_deadline_is_contract_timestamp_not_wall_sleep() {
    let set = registration_set(vec![registration(
        "reg:sms-1",
        "sms_inbound",
        "sms:inbox-1",
    )]);
    let mut request = live_wait_request();
    request.logical_deadline = ts("2026-08-15T16:02:00.123456Z");
    request.run_deadline = request.logical_deadline.clone();
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = IdleObserver::new();
    let outcome = run_first_match(&set, &request, &observer, &clock, &Cancel::new())
        .await
        .expect("logical deadline");
    admit_live(&outcome);
    assert_eq!(outcome.outcome_kind, OutcomeKind::LogicalDeadman);
    assert_eq!(
        outcome.completed_at.as_str(),
        "2026-08-15T16:02:00.123456Z",
        "completed_at is the contract deadline, not a wall Instant"
    );
    assert_eq!(clock.current_time(), request.logical_deadline);
    assert_eq!(observer.live_bind_count(), 0);
}

#[tokio::test]
async fn poll_ack_same_observer_replays_until_ack() {
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
        .expect("first cycle");
    admit_poll(&first);
    assert_eq!(first.events[0].event_id.as_str(), "evt:sms-1");
    let start = first
        .arms
        .iter()
        .find(|arm| arm.registration_id.as_str() == "reg:sms-1")
        .map(|arm| arm.start_anchor.clone())
        .expect("start");
    let proposed = first
        .proposed_next_anchors
        .get("reg:sms-1")
        .expect("proposed")
        .clone();
    assert_ne!(proposed, start);
    assert_eq!(
        observer
            .queued_event_ids()
            .get("reg:sms-1")
            .map(Vec::as_slice),
        Some(["evt:sms-1".to_string()].as_slice()),
        "same observer must still hold the admitted event before ack"
    );

    let mut replay_req = request.clone();
    replay_req.message_id = IdToken::new("msg:aw-poll-req-2");
    let replay = run_poll_cycle(&set, &replay_req, &observer, &clock, &Cancel::new())
        .await
        .expect("unacked same-observer replay");
    admit_poll(&replay);
    assert_eq!(replay.events[0].event_id.as_str(), "evt:sms-1");
    assert_eq!(
        replay
            .arms
            .iter()
            .find(|arm| arm.registration_id.as_str() == "reg:sms-1")
            .map(|arm| &arm.start_anchor),
        Some(&start),
        "unacked same-observer restart must keep the original start"
    );

    let ack = ack_poll_outcome(&first);
    let mut committed_req = request.clone();
    committed_req.message_id = IdToken::new("msg:aw-poll-req-3");
    committed_req.acknowledged_anchors = ack.committed_anchors.clone();
    let committed = run_poll_cycle(&set, &committed_req, &observer, &clock, &Cancel::new())
        .await
        .expect("acked cycle on the same observer");
    admit_poll(&committed);
    assert_eq!(
        observer.bind_requested_starts().get("reg:sms-1"),
        Some(&Some(proposed.clone())),
        "poll_cycle_ack on the same observer commits the cursor"
    );
    assert_eq!(
        committed
            .arms
            .iter()
            .find(|arm| arm.registration_id.as_str() == "reg:sms-1")
            .map(|arm| &arm.start_anchor),
        Some(&proposed)
    );
}

async fn prove_cancel_is_watch_token_not_job_object() {
    let cancel = Cancel::new();
    assert!(!cancel.is_cancelled());
    let waiter = {
        let cancel = cancel.clone();
        tokio::spawn(async move {
            cancel.cancelled().await;
        })
    };
    tokio::task::yield_now().await;
    cancel.trigger();
    waiter.await.expect("join");
    assert!(cancel.is_cancelled());
}

#[tokio::test]
async fn portable_cancel_is_watch_token_not_job_object() {
    prove_cancel_is_watch_token_not_job_object().await;
}

#[cfg(windows)]
#[tokio::test]
async fn windows_portable_cancel_is_not_a_job_object() {
    prove_cancel_is_watch_token_not_job_object().await;
}
