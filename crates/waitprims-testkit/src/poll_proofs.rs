//! Proofs: poll-cycle coverage, fairness, ack, replay, retention, starvation, dirty arms.

use std::collections::BTreeMap;
use std::path::PathBuf;

use waitprims_async::{event_surface_bytes, run_poll_cycle, Cancel, POLL_ACK_RETENTION};
use waitprims_core::{
    validate_message, validate_raw_documents, AgentWaitMessage, Anchor, AnchorKind, ArmStatus,
    IdToken, MessageType, OutcomeKind, PollBound, PollCycleAck, PollCycleOutcome, Timestamp,
};

use crate::{
    ack_poll_outcome, exclusive_head_anchor, poll_cycle_request, registration,
    registration_baseline, registration_set, ts, wait_event, EndlessReadyObserver, FakeClock,
    IdleObserver, Script, ScriptedObserver,
};

fn admit(outcome: &PollCycleOutcome) {
    let message = AgentWaitMessage::PollCycleOutcome(outcome.clone());
    let json = serde_json::to_string(&message).expect("serialize outcome");
    validate_message(&json).unwrap_or_else(|err| panic!("outcome must admit: {err}; {json}"));
}

fn admit_pair(outcome: &PollCycleOutcome, ack: &PollCycleAck) {
    let out = serde_json::to_string(&AgentWaitMessage::PollCycleOutcome(outcome.clone()))
        .expect("serialize outcome");
    let ack =
        serde_json::to_string(&AgentWaitMessage::PollCycleAck(ack.clone())).expect("serialize ack");
    validate_raw_documents([&out, &ack]).unwrap_or_else(|err| panic!("ack set must admit: {err}"));
}

fn vendor_set(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../schemas/v0/rejects/set")
        .join(name)
}

fn load_json_dir(dir: &PathBuf) -> Vec<String> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir).expect("read dir") {
        let path = entry.expect("entry").path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
            files.push(path);
        }
    }
    files.sort();
    files
        .iter()
        .map(|path| std::fs::read_to_string(path).expect("read json"))
        .collect()
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

fn two_arm_set() -> waitprims_core::RegistrationSet {
    registration_set(vec![
        registration("reg:chanvoy-1", "chanvoy_wait", "chan:seat-a"),
        registration("reg:sms-1", "sms_inbound", "sms:inbox-1"),
    ])
}

fn live_script() -> Script {
    Script {
        buffer_limit: 8,
        events: vec![
            wait_event(
                "reg:sms-1",
                "sms_inbound",
                "evt:sms-1",
                "2026-08-15T16:05:00Z",
            ),
            wait_event(
                "reg:chanvoy-1",
                "chanvoy_wait",
                "evt:chanvoy-1",
                "2026-08-15T16:10:00Z",
            ),
            wait_event(
                "reg:job-1",
                "job_complete",
                "evt:job-1",
                "2026-08-15T16:12:00Z",
            ),
        ],
    }
}

async fn run_cycle(
    set: &waitprims_core::RegistrationSet,
    request: &waitprims_core::PollCycleRequest,
    observer: &ScriptedObserver,
    clock: &FakeClock,
) -> PollCycleOutcome {
    let outcome = run_poll_cycle(set, request, observer, clock, &Cancel::new())
        .await
        .expect("poll cycle");
    admit(&outcome);
    assert_eq!(observer.live_bind_count(), 0);
    let json = serde_json::to_string(&outcome).expect("serialize");
    assert!(
        !json.contains("anc:baseline-latest"),
        "must not mint a policy label as a cursor: {json}"
    );
    outcome
}

#[tokio::test]
async fn required_arms_covered_fairness_rotates_deadlines_honored() {
    let set = three_arm_set();
    let request = poll_cycle_request(&set);
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = ScriptedObserver::new(live_script(), clock.clone());
    let outcome = run_cycle(&set, &request, &observer, &clock).await;
    assert_eq!(outcome.outcome_kind, OutcomeKind::Events);
    assert!(outcome.coverage_complete);
    assert_eq!(outcome.arms.len(), 3);
    for required in &request.required_arms {
        assert!(
            outcome
                .arms
                .iter()
                .any(|arm| arm.arm_id.as_str() == required.as_str()),
            "missing required arm {}",
            required.as_str()
        );
    }
    assert_eq!(outcome.fairness_cursor.as_str(), "fair:start");
    assert_ne!(
        outcome.next_fairness_cursor.as_str(),
        outcome.fairness_cursor.as_str(),
        "fairness_cursor must rotate"
    );
    assert!(outcome.next_fairness_cursor.as_str().starts_with("fair:"));
    assert_eq!(outcome.events.len(), 3);

    let mut short = poll_cycle_request(&set);
    short.message_id = IdToken::new("msg:aw-poll-req-short");
    short.run_deadline = ts("2026-08-15T16:06:00Z");
    let clock = FakeClock::auto(short.created_at.clone());
    let observer = ScriptedObserver::new(live_script(), clock.clone());
    let leftover = run_cycle(&set, &short, &observer, &clock).await;
    assert!(
        leftover.completed_at <= short.run_deadline,
        "run_deadline must win leftover work"
    );
    assert_eq!(leftover.completed_at, ts("2026-08-15T16:06:00Z"));
    assert!(
        leftover
            .arms
            .iter()
            .any(|arm| arm.status == ArmStatus::Deferred),
        "leftover arms at run_deadline must defer: {:?}",
        leftover.arms
    );
    assert_eq!(leftover.outcome_kind, OutcomeKind::Partial);
    assert!(!leftover.coverage_complete);

    let mut logical = poll_cycle_request(&set);
    logical.message_id = IdToken::new("msg:aw-poll-req-logical");
    logical.logical_deadline = ts("2026-08-15T16:02:00Z");
    logical.run_deadline = logical.logical_deadline.clone();
    let clock = FakeClock::auto(logical.created_at.clone());
    let observer = ScriptedObserver::new(Script::default(), clock.clone());
    let deadman = run_cycle(&set, &logical, &observer, &clock).await;
    assert_eq!(deadman.outcome_kind, OutcomeKind::LogicalDeadman);
    assert_eq!(
        deadman.completed_at,
        Timestamp::parse("2026-08-15T16:02:00Z").unwrap()
    );
    assert!(clock.current_time() <= logical.logical_deadline);
    assert!(deadman.coverage_complete);
    assert!(deadman.events.is_empty());
}

#[tokio::test]
async fn empty_cycle_is_no_change_before_logical_deadline() {
    let set = three_arm_baseline_set();
    let request = poll_cycle_request(&set);
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = ScriptedObserver::new(Script::default(), clock.clone());
    let outcome = run_cycle(&set, &request, &observer, &clock).await;
    assert_eq!(outcome.outcome_kind, OutcomeKind::NoChange);
    assert_eq!(outcome.completed_at, ts("2026-08-15T16:20:00Z"));
    assert_eq!(outcome.logical_deadline.as_str(), "2026-08-15T17:00:00Z");
    assert!(outcome.coverage_complete);
    assert_eq!(outcome.arms.len(), 3);
    for arm in &outcome.arms {
        assert_eq!(arm.status, ArmStatus::NoChange);
        assert_ne!(arm.start_anchor.value.as_str(), "anc:baseline-latest");
    }
    assert_eq!(
        exclusive_head_anchor(&set.registrations[0].registration_id)
            .value
            .as_str(),
        "anc:h-chanvoy-1"
    );
}

#[tokio::test]
async fn idle_observer_honors_logical_deadline() {
    let set = three_arm_set();
    let mut request = poll_cycle_request(&set);
    request.logical_deadline = ts("2026-08-15T16:02:00Z");
    request.run_deadline = request.logical_deadline.clone();
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = IdleObserver::new();
    let outcome = run_poll_cycle(&set, &request, &observer, &clock, &Cancel::new())
        .await
        .expect("idle snapshot");
    admit(&outcome);
    assert_eq!(
        outcome.outcome_kind,
        OutcomeKind::NoChange,
        "immediate idle is a clean snapshot before logical_deadline"
    );
    assert!(outcome.completed_at < request.logical_deadline);
    assert_eq!(observer.live_bind_count(), 0);
}

#[tokio::test]
async fn ack_resolves_outcome_and_keeps_same_arm_maps() {
    let set = three_arm_set();
    let request = poll_cycle_request(&set);
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = ScriptedObserver::new(live_script(), clock.clone());
    let outcome = run_cycle(&set, &request, &observer, &clock).await;
    let ack = ack_poll_outcome(&outcome);
    assert_eq!(ack.outcome_ref.as_str(), outcome.message_id.as_str());
    for (rid, anchor) in &ack.committed_anchors {
        assert!(
            set.registrations
                .iter()
                .any(|reg| reg.registration_id.as_str() == rid),
            "committed_anchors must be keyed by registration_id: {rid}"
        );
        assert_eq!(
            outcome.retained_through.get(rid),
            Some(anchor),
            "committed_anchors must equal retained_through for {rid}"
        );
    }
    for (rid, events) in &ack.retained_events {
        let allowed = outcome
            .retained_events
            .get(rid)
            .cloned()
            .unwrap_or_default();
        for event_id in events {
            assert!(
                allowed
                    .iter()
                    .any(|have| have.as_str() == event_id.as_str()),
                "retained event {} is not a same-arm subset",
                event_id.as_str()
            );
        }
    }
    admit_pair(&outcome, &ack);
}

#[tokio::test]
async fn cross_arm_and_past_unretained_commits_reject() {
    let set = two_arm_set();
    let request = poll_cycle_request(&set);
    let script = Script {
        buffer_limit: 8,
        events: vec![
            wait_event(
                "reg:chanvoy-1",
                "chanvoy_wait",
                "evt:chanvoy-1",
                "2026-08-15T16:05:00Z",
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
    let outcome = run_cycle(&set, &request, &observer, &clock).await;
    let out_json = serde_json::to_string(&AgentWaitMessage::PollCycleOutcome(outcome.clone()))
        .expect("serialize outcome");

    let mut stolen = ack_poll_outcome(&outcome);
    if let (Some(left), Some(right)) = (
        outcome.retained_through.get("reg:chanvoy-1").cloned(),
        outcome.retained_through.get("reg:sms-1").cloned(),
    ) {
        stolen
            .committed_anchors
            .insert("reg:chanvoy-1".to_string(), right);
        stolen
            .committed_anchors
            .insert("reg:sms-1".to_string(), left);
    }
    if let Some(sms_events) = outcome.retained_events.get("reg:sms-1").cloned() {
        stolen
            .retained_events
            .insert("reg:chanvoy-1".to_string(), sms_events);
    }
    let stolen_json =
        serde_json::to_string(&AgentWaitMessage::PollCycleAck(stolen)).expect("serialize stolen");
    let err = validate_raw_documents([&out_json, &stolen_json]).expect_err("cross-arm must reject");
    assert!(
        err.to_string().contains("cross_registration")
            || err.to_string().contains("past_unretained"),
        "unexpected cross-arm error: {err}"
    );

    let mut past = ack_poll_outcome(&outcome);
    past.committed_anchors.insert(
        "reg:sms-1".to_string(),
        Anchor {
            kind: AnchorKind::ProviderOpaque,
            value: IdToken::new("anc:past-unretained"),
        },
    );
    let past_json =
        serde_json::to_string(&AgentWaitMessage::PollCycleAck(past)).expect("serialize past");
    let err =
        validate_raw_documents([&out_json, &past_json]).expect_err("past-unretained must reject");
    assert!(
        err.to_string().contains("past_unretained"),
        "unexpected past-unretained error: {err}"
    );
}

#[tokio::test]
async fn without_ack_events_may_replay_cursors_do_not_silently_advance() {
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
    let first_observer = ScriptedObserver::new(script.clone(), clock.clone());
    let first = run_cycle(&set, &request, &first_observer, &clock).await;
    assert_eq!(first.events[0].event_id.as_str(), "evt:sms-1");
    let proposed = first
        .proposed_next_anchors
        .get("reg:sms-1")
        .expect("proposed")
        .clone();

    let clock = FakeClock::auto(request.created_at.clone());
    let replay_observer = ScriptedObserver::new(script, clock.clone());
    let replay = run_cycle(&set, &request, &replay_observer, &clock).await;
    assert_eq!(replay.events[0].event_id.as_str(), "evt:sms-1");
    assert_eq!(
        replay.proposed_next_anchors.get("reg:sms-1"),
        Some(&proposed)
    );
    let first_json = serde_json::to_string(&AgentWaitMessage::PollCycleOutcome(first.clone()))
        .expect("serialize first");
    let replay_json = serde_json::to_string(&AgentWaitMessage::PollCycleOutcome(replay.clone()))
        .expect("serialize replay");
    validate_raw_documents([&first_json, &replay_json])
        .expect("stable event ids may replay without ack");

    let clock = FakeClock::auto(request.created_at.clone());
    let consumed = ScriptedObserver::new(Script::default(), clock.clone());
    let second = run_cycle(&set, &request, &consumed, &clock).await;
    assert!(second.events.is_empty());
    let start = second
        .proposed_next_anchors
        .get("reg:sms-1")
        .expect("start cursor");
    assert_ne!(
        start.value.as_str(),
        proposed.value.as_str(),
        "unacked second cycle must not inherit the first proposed cursor"
    );
    assert_eq!(start.value.as_str(), "anc:cursor-0");

    let reject_dir = vendor_set("reject-silent-cursor-advance");
    let err = validate_raw_documents(load_json_dir(&reject_dir))
        .expect_err("pinned silent-advance control must reject");
    assert!(
        err.to_string().contains("silent_advance"),
        "unexpected silent-advance error: {err}"
    );
    let baseline = vendor_set("baseline-replay-ids");
    validate_raw_documents(load_json_dir(&baseline)).expect("replay baseline must admit");
}

#[tokio::test]
async fn required_arm_may_defer_once_then_fairness_rotates() {
    let set = two_arm_set();
    let mut first_req = poll_cycle_request(&set);
    first_req.run_deadline = ts("2026-08-15T16:06:00Z");
    let noisy = Script {
        buffer_limit: 8,
        events: vec![
            wait_event(
                "reg:chanvoy-1",
                "chanvoy_wait",
                "evt:chanvoy-1",
                "2026-08-15T16:05:00Z",
            ),
            wait_event(
                "reg:sms-1",
                "sms_inbound",
                "evt:sms-1",
                "2026-08-15T16:10:00Z",
            ),
        ],
    };
    let clock = FakeClock::auto(first_req.created_at.clone());
    let observer = ScriptedObserver::new(noisy.clone(), clock.clone());
    let first = run_cycle(&set, &first_req, &observer, &clock).await;
    assert_eq!(first.outcome_kind, OutcomeKind::Partial);
    let deferred = first
        .arms
        .iter()
        .find(|arm| arm.status == ArmStatus::Deferred)
        .expect("one required arm may defer");
    assert_eq!(deferred.registration_id.as_str(), "reg:sms-1");
    assert_ne!(
        first.next_fairness_cursor.as_str(),
        first.fairness_cursor.as_str()
    );

    let mut second_req = first_req.clone();
    second_req.message_id = IdToken::new("msg:aw-poll-req-2");
    second_req.fairness_cursor = first.next_fairness_cursor.clone();
    second_req.run_deadline = ts("2026-08-15T16:20:00Z");
    let clock = FakeClock::auto(second_req.created_at.clone());
    let observer = ScriptedObserver::new(noisy, clock.clone());
    let second = run_cycle(&set, &second_req, &observer, &clock).await;
    assert_ne!(
        second.next_fairness_cursor.as_str(),
        second.fairness_cursor.as_str(),
        "successive noisy cycles must rotate next_fairness_cursor"
    );
    assert!(
        second
            .arms
            .iter()
            .any(|arm| arm.registration_id.as_str() == "reg:sms-1"
                && arm.status != ArmStatus::Deferred),
        "rotated cycle must visit the previously deferred arm: {:?}",
        second.arms
    );

    let first_json =
        serde_json::to_string(&AgentWaitMessage::PollCycleOutcome(first)).expect("serialize first");
    let second_json = serde_json::to_string(&AgentWaitMessage::PollCycleOutcome(second))
        .expect("serialize second");
    validate_raw_documents([&first_json, &second_json])
        .expect("rotating fairness must not trip starvation");

    let reject_dir = vendor_set("reject-fairness-starvation");
    let err = validate_raw_documents(load_json_dir(&reject_dir))
        .expect_err("pinned starvation control must reject");
    assert!(
        err.to_string().contains("starvation"),
        "unexpected starvation error: {err}"
    );
    let baseline = vendor_set("baseline-fairness-rotation");
    validate_raw_documents(load_json_dir(&baseline))
        .expect("fairness rotation baseline must admit");
}

#[tokio::test]
async fn dirty_required_arm_is_not_clean_complete() {
    let set = three_arm_set();
    let request = poll_cycle_request(&set);
    for (label, inject) in [
        (
            "outage",
            Box::new(|observer: &ScriptedObserver| observer.outage("reg:sms-1", "provider_outage"))
                as Box<dyn Fn(&ScriptedObserver)>,
        ),
        (
            "cursor_uncertain",
            Box::new(|observer: &ScriptedObserver| {
                observer.cursor_uncertain("reg:sms-1", "cursor_uncertain")
            }),
        ),
        (
            "degraded",
            Box::new(|observer: &ScriptedObserver| observer.degrade("reg:sms-1", "arm_degraded")),
        ),
    ] {
        let clock = FakeClock::auto(request.created_at.clone());
        let observer = ScriptedObserver::new(Script::default(), clock.clone());
        inject(&observer);
        let outcome = run_cycle(&set, &request, &observer, &clock).await;
        assert_ne!(
            outcome.outcome_kind,
            OutcomeKind::NoChange,
            "{label} must not be no_change"
        );
        assert_ne!(
            outcome.outcome_kind,
            OutcomeKind::LogicalDeadman,
            "{label} must not be logical_deadman"
        );
        assert_eq!(outcome.outcome_kind, OutcomeKind::CoverageDegraded);
        assert!(!outcome.coverage_complete);
        let dirty = outcome
            .arms
            .iter()
            .find(|arm| arm.registration_id.as_str() == "reg:sms-1")
            .expect("dirty arm");
        match label {
            "outage" => assert_eq!(dirty.status, ArmStatus::Outage),
            "cursor_uncertain" => assert_eq!(dirty.status, ArmStatus::CursorUncertain),
            "degraded" => assert!(dirty.degraded),
            _ => unreachable!(),
        }
        assert!(dirty.reason_code.is_some());
    }

    let mut logical = poll_cycle_request(&set);
    logical.message_id = IdToken::new("msg:aw-poll-req-dirty-logical");
    logical.logical_deadline = ts("2026-08-15T16:02:00Z");
    logical.run_deadline = logical.logical_deadline.clone();
    let clock = FakeClock::auto(logical.created_at.clone());
    let observer = ScriptedObserver::new(Script::default(), clock.clone());
    observer.outage("reg:chanvoy-1", "provider_outage");
    let outcome = run_cycle(&set, &logical, &observer, &clock).await;
    assert_ne!(outcome.outcome_kind, OutcomeKind::LogicalDeadman);
    assert_ne!(outcome.outcome_kind, OutcomeKind::NoChange);
    assert_eq!(outcome.outcome_kind, OutcomeKind::CoverageDegraded);
}

#[tokio::test]
async fn hanging_required_bind_is_failed_not_clean() {
    let set = three_arm_baseline_set();
    let request = poll_cycle_request(&set);
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = ScriptedObserver::new(Script::default(), clock.clone());
    observer.hang_bind("reg:chanvoy-1");
    let outcome = run_poll_cycle(&set, &request, &observer, &clock, &Cancel::new())
        .await
        .expect("terminal");
    admit(&outcome);
    assert_eq!(outcome.outcome_kind, OutcomeKind::Failed);
    assert_eq!(
        outcome.reason_code.as_ref().map(IdToken::as_str),
        Some("required_bind_pending")
    );
    assert_ne!(outcome.outcome_kind, OutcomeKind::NoChange);
    assert_ne!(outcome.outcome_kind, OutcomeKind::LogicalDeadman);
    assert!(!outcome.coverage_complete);
    assert_eq!(observer.live_bind_count(), 0);
    assert!(
        outcome
            .arms
            .iter()
            .all(|arm| arm.registration_id.as_str() != "reg:chanvoy-1"),
        "pending baseline bind must not emit a fabricated arm: {:?}",
        outcome.arms
    );
    let json = serde_json::to_string(&outcome).expect("serialize");
    assert!(
        !json.contains("anc:h-chanvoy-1"),
        "pending baseline bind must not invent anc:h-…: {json}"
    );
}

#[tokio::test]
async fn hanging_cancel_does_not_delay_poll_outcome() {
    let set = two_arm_set();
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
    let observer = ScriptedObserver::new(script, clock.clone());
    observer.hang_cancel();
    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        run_poll_cycle(&set, &request, &observer, &clock, &Cancel::new()),
    )
    .await
    .expect("hung cancel must not delay a decided outcome")
    .expect("runner");
    admit(&outcome);
    assert_eq!(observer.live_bind_count(), 0);
}

#[tokio::test]
async fn cancel_during_poll_is_cancelled() {
    let set = three_arm_set();
    let request = poll_cycle_request(&set);
    let clock = FakeClock::manual(request.created_at.clone());
    let observer = ScriptedObserver::new(Script::default(), clock.clone());
    let cancel = Cancel::new();
    let set2 = set.clone();
    let request2 = request.clone();
    let clock2 = clock.clone();
    let observer2 = observer.clone();
    let cancel2 = cancel.clone();
    let task = tokio::spawn(async move {
        run_poll_cycle(&set2, &request2, &observer2, &clock2, &cancel2).await
    });
    while clock.sleeper_count() == 0 {
        tokio::task::yield_now().await;
    }
    cancel.trigger();
    let outcome = task.await.expect("join").expect("cancelled");
    admit(&outcome);
    assert_eq!(outcome.outcome_kind, OutcomeKind::Cancelled);
    assert_eq!(observer.live_bind_count(), 0);
}

#[tokio::test]
async fn fairness_cursor_rotates_scarce_capacity() {
    let set = two_arm_set();
    let at = "2026-08-15T16:05:00Z";
    let noisy = Script {
        buffer_limit: 8,
        events: vec![
            wait_event("reg:chanvoy-1", "chanvoy_wait", "evt:chanvoy-1", at),
            wait_event("reg:sms-1", "sms_inbound", "evt:sms-1", at),
        ],
    };
    let mut first = poll_cycle_request(&set);
    first.bound = Some(PollBound {
        max_events: Some(1),
        max_payload_refs: None,
        max_bytes: None,
    });
    first.fairness_cursor = IdToken::new("fair:start");
    let mut second = first.clone();
    second.message_id = IdToken::new("msg:aw-poll-req-2");
    second.fairness_cursor = IdToken::new("fair:arm:sms-1");
    assert_eq!(first.run_deadline, second.run_deadline);
    assert_eq!(first.bound, second.bound);
    assert_eq!(first.logical_deadline, second.logical_deadline);
    assert_ne!(first.fairness_cursor, second.fairness_cursor);

    let clock = FakeClock::auto(first.created_at.clone());
    let observer = ScriptedObserver::new(noisy.clone(), clock.clone());
    let left = run_cycle(&set, &first, &observer, &clock).await;
    let clock = FakeClock::auto(second.created_at.clone());
    let observer = ScriptedObserver::new(noisy, clock.clone());
    let right = run_cycle(&set, &second, &observer, &clock).await;

    assert_eq!(left.events.len(), 1);
    assert_eq!(right.events.len(), 1);
    assert_eq!(left.events[0].registration_id.as_str(), "reg:chanvoy-1");
    assert_eq!(right.events[0].registration_id.as_str(), "reg:sms-1");
    assert_eq!(
        left.arms
            .iter()
            .find(|arm| arm.registration_id.as_str() == "reg:sms-1")
            .map(|arm| arm.status),
        Some(ArmStatus::Deferred)
    );
    assert_eq!(
        right
            .arms
            .iter()
            .find(|arm| arm.registration_id.as_str() == "reg:chanvoy-1")
            .map(|arm| arm.status),
        Some(ArmStatus::Deferred)
    );
    assert_eq!(left.outcome_kind, OutcomeKind::Partial);
    assert_eq!(right.outcome_kind, OutcomeKind::Partial);
}

#[tokio::test]
async fn acknowledged_anchor_is_the_bind_start() {
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
    let first_observer = ScriptedObserver::new(script, clock.clone());
    let first = run_cycle(&set, &request, &first_observer, &clock).await;
    let ack = ack_poll_outcome(&first);
    let committed = ack
        .committed_anchors
        .get("reg:sms-1")
        .expect("committed")
        .clone();
    assert_ne!(committed.value.as_str(), "anc:cursor-0");

    let mut second_req = request.clone();
    second_req.message_id = IdToken::new("msg:aw-poll-req-2");
    second_req.acknowledged_anchors = ack.committed_anchors.clone();
    let clock = FakeClock::auto(second_req.created_at.clone());
    let second_observer = ScriptedObserver::new(Script::default(), clock.clone());
    let second = run_cycle(&set, &second_req, &second_observer, &clock).await;
    assert_eq!(
        second_observer.bind_requested_starts().get("reg:sms-1"),
        Some(&Some(committed.clone()))
    );
    assert_eq!(
        second_observer.bind_resolved_starts().get("reg:sms-1"),
        Some(&committed)
    );
    assert_eq!(
        second
            .arms
            .iter()
            .find(|arm| arm.registration_id.as_str() == "reg:sms-1")
            .map(|arm| &arm.start_anchor),
        Some(&committed)
    );

    let mut unknown = request.clone();
    unknown
        .acknowledged_anchors
        .insert("reg:not-in-set".to_string(), committed.clone());
    let clock = FakeClock::auto(unknown.created_at.clone());
    let observer = ScriptedObserver::new(Script::default(), clock.clone());
    let err = run_poll_cycle(&set, &unknown, &observer, &clock, &Cancel::new())
        .await
        .expect_err("unknown ack key");
    assert!(
        err.to_string().contains("unknown_registration"),
        "unexpected unknown-ack error: {err}"
    );
}

#[tokio::test]
async fn pending_baseline_bind_does_not_invent_anchor() {
    let set = three_arm_baseline_set();
    let request = poll_cycle_request(&set);
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = ScriptedObserver::new(Script::default(), clock.clone());
    for registration in &set.registrations {
        observer.hang_bind(registration.registration_id.as_str());
    }
    let err = run_poll_cycle(&set, &request, &observer, &clock, &Cancel::new())
        .await
        .expect_err("no honest start without bind");
    assert!(
        err.to_string().contains("unresolved_start")
            || err.to_string().contains("required_bind_pending"),
        "unexpected pending-bind error: {err}"
    );
    assert!(!err.to_string().contains("anc:h-"));
}

#[tokio::test]
async fn per_registration_event_bound_is_partial_not_complete() {
    let mut reg = registration("reg:sms-1", "sms_inbound", "sms:inbox-1");
    reg.bounds.max_events = 1;
    let set = registration_set(vec![reg]);
    let request = poll_cycle_request(&set);
    assert!(
        request.bound.is_none(),
        "proof must not lean on a request-level event cap"
    );
    assert_eq!(set.registrations.len(), 1);
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = EndlessReadyObserver::new();
    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        run_poll_cycle(&set, &request, &observer, &clock, &Cancel::new()),
    )
    .await
    .expect("per-registration event bound must stop drain")
    .expect("poll cycle");
    admit(&outcome);
    assert_eq!(outcome.events.len(), 1);
    assert_eq!(outcome.outcome_kind, OutcomeKind::Partial);
    assert!(!outcome.coverage_complete);
    assert_eq!(outcome.arms.len(), 1);
    assert_eq!(outcome.arms[0].status, ArmStatus::Events);
    assert_eq!(outcome.arms[0].event_count, 1);
    assert_eq!(
        outcome.arms[0].reason_code.as_ref().map(IdToken::as_str),
        Some("bound_exhausted")
    );
    assert_eq!(observer.live_bind_count(), 0);

    let mut replay_reg = registration("reg:sms-1", "sms_inbound", "sms:inbox-1");
    replay_reg.bounds.max_events = 1;
    let replay_set = registration_set(vec![replay_reg]);
    let replay_req = poll_cycle_request(&replay_set);
    let script = Script {
        buffer_limit: 8,
        events: vec![
            wait_event(
                "reg:sms-1",
                "sms_inbound",
                "evt:sms-1",
                "2026-08-15T16:05:00Z",
            ),
            wait_event(
                "reg:sms-1",
                "sms_inbound",
                "evt:sms-2",
                "2026-08-15T16:05:00Z",
            ),
        ],
    };
    let clock = FakeClock::auto(replay_req.created_at.clone());
    let observer = ScriptedObserver::new(script, clock.clone());
    let first = run_cycle(&replay_set, &replay_req, &observer, &clock).await;
    assert_eq!(first.outcome_kind, OutcomeKind::Partial);
    assert_eq!(first.events.len(), 1);
    assert_eq!(first.events[0].event_id.as_str(), "evt:sms-1");
    assert_eq!(
        observer
            .queued_event_ids()
            .get("reg:sms-1")
            .map(Vec::as_slice),
        Some(["evt:sms-1".to_string(), "evt:sms-2".to_string()].as_slice()),
        "admitted and leftover events must both stay queued until ack: {:?}",
        observer.queued_event_ids()
    );
    let mut second_req = replay_req.clone();
    second_req.message_id = IdToken::new("msg:aw-poll-req-2");
    let second = run_cycle(&replay_set, &second_req, &observer, &clock).await;
    assert_eq!(
        second
            .events
            .iter()
            .map(|e| e.event_id.as_str())
            .collect::<Vec<_>>(),
        vec!["evt:sms-1"],
        "unacked restart on the same observer must replay the admitted event: {:?}",
        second.events
    );
}

#[tokio::test]
async fn per_registration_byte_bound_is_partial_not_complete() {
    let sample = wait_event(
        "reg:sms-1",
        "sms_inbound",
        "evt:sample",
        "2026-08-15T16:01:00Z",
    );
    let surface = event_surface_bytes(&sample);
    let mut reg = registration("reg:sms-1", "sms_inbound", "sms:inbox-1");
    reg.bounds.max_bytes = surface;
    let set = registration_set(vec![reg]);
    let request = poll_cycle_request(&set);
    assert!(
        request.bound.is_none(),
        "proof must not lean on a request-level byte cap"
    );
    assert_eq!(set.registrations.len(), 1);
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = EndlessReadyObserver::new();
    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        run_poll_cycle(&set, &request, &observer, &clock, &Cancel::new()),
    )
    .await
    .expect("per-registration byte bound must stop drain")
    .expect("poll cycle");
    admit(&outcome);
    assert_eq!(outcome.events.len(), 1);
    assert_eq!(outcome.outcome_kind, OutcomeKind::Partial);
    assert!(!outcome.coverage_complete);
    assert_eq!(outcome.arms.len(), 1);
    assert_eq!(outcome.arms[0].status, ArmStatus::Events);
    assert_eq!(outcome.arms[0].byte_count, surface);
    assert_eq!(
        outcome.arms[0].reason_code.as_ref().map(IdToken::as_str),
        Some("bound_exhausted")
    );
    assert_eq!(observer.live_bind_count(), 0);
}

#[tokio::test]
async fn pre_bind_cancel_keeps_explicit_start() {
    let set = three_arm_set();
    let request = poll_cycle_request(&set);
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = ScriptedObserver::new(Script::default(), clock.clone());
    let cancel = Cancel::new();
    cancel.trigger();
    let outcome = run_poll_cycle(&set, &request, &observer, &clock, &cancel)
        .await
        .expect("pre-bind cancel must admit cancelled, not unresolved_start");
    admit(&outcome);
    assert_eq!(outcome.outcome_kind, OutcomeKind::Cancelled);
    assert_eq!(
        outcome.reason_code.as_ref().map(IdToken::as_str),
        Some("consumer_cancelled")
    );
    assert_eq!(outcome.arms.len(), 3);
    for arm in &outcome.arms {
        assert_eq!(arm.start_anchor.value.as_str(), "anc:cursor-0");
        assert!(!arm.start_anchor.value.as_str().starts_with("anc:h-"));
    }
    let json = serde_json::to_string(&outcome).expect("serialize");
    assert!(!json.contains("anc:h-"), "must not mint anc:h-…: {json}");
    assert_eq!(observer.live_bind_count(), 0);
    assert!(observer.bind_requested_starts().is_empty());
}

#[tokio::test]
async fn entry_deadline_keeps_explicit_and_acked_starts() {
    let set = three_arm_set();
    let mut request = poll_cycle_request(&set);
    request.run_deadline = request.created_at.clone();
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = ScriptedObserver::new(Script::default(), clock.clone());
    let outcome = run_poll_cycle(&set, &request, &observer, &clock, &Cancel::new())
        .await
        .expect("entry deadline must admit failed, not unresolved_start");
    admit(&outcome);
    assert_eq!(outcome.outcome_kind, OutcomeKind::Failed);
    assert_eq!(
        outcome.reason_code.as_ref().map(IdToken::as_str),
        Some("required_bind_pending")
    );
    assert!(!outcome.coverage_complete);
    assert_eq!(outcome.arms.len(), 3);
    for arm in &outcome.arms {
        assert_eq!(arm.start_anchor.value.as_str(), "anc:cursor-0");
    }
    assert!(observer.bind_requested_starts().is_empty());

    let acked = Anchor {
        kind: AnchorKind::ProviderOpaque,
        value: IdToken::new("anc:acked-1"),
    };
    let mut acked_req = request.clone();
    acked_req.message_id = IdToken::new("msg:aw-poll-req-acked");
    acked_req
        .acknowledged_anchors
        .insert("reg:sms-1".to_string(), acked.clone());
    let clock = FakeClock::auto(acked_req.created_at.clone());
    let observer = ScriptedObserver::new(Script::default(), clock.clone());
    let acked_out = run_poll_cycle(&set, &acked_req, &observer, &clock, &Cancel::new())
        .await
        .expect("acked explicit start is contract evidence");
    admit(&acked_out);
    assert_eq!(acked_out.outcome_kind, OutcomeKind::Failed);
    assert_eq!(
        acked_out.reason_code.as_ref().map(IdToken::as_str),
        Some("required_bind_pending")
    );
    assert_eq!(
        acked_out
            .arms
            .iter()
            .find(|arm| arm.registration_id.as_str() == "reg:sms-1")
            .map(|arm| &arm.start_anchor),
        Some(&acked)
    );
    assert_eq!(
        acked_out
            .arms
            .iter()
            .find(|arm| arm.registration_id.as_str() == "reg:chanvoy-1")
            .map(|arm| arm.start_anchor.value.as_str()),
        Some("anc:cursor-0")
    );
    let json = serde_json::to_string(&acked_out).expect("serialize");
    assert!(!json.contains("anc:h-"), "must not mint anc:h-…: {json}");
}

#[tokio::test]
async fn collection_stops_at_payload_ref_and_byte_bounds() {
    let set = two_arm_set();
    let sample = wait_event(
        "reg:chanvoy-1",
        "chanvoy_wait",
        "evt:sample",
        "2026-08-15T16:01:00Z",
    );
    let surface = event_surface_bytes(&sample);
    assert!(surface > 0, "surface bytes are the observable contract");

    let mut req = poll_cycle_request(&set);
    req.bound = Some(PollBound {
        max_events: None,
        max_payload_refs: Some(1),
        max_bytes: None,
    });
    let clock = FakeClock::auto(req.created_at.clone());
    let observer = EndlessReadyObserver::new();
    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        run_poll_cycle(&set, &req, &observer, &clock, &Cancel::new()),
    )
    .await
    .expect("endless ready must not unbounded-drain")
    .expect("poll cycle");
    admit(&outcome);
    assert_eq!(outcome.events.len(), 1);
    assert_eq!(outcome.outcome_kind, OutcomeKind::Partial);
    assert_eq!(observer.live_bind_count(), 0);
    assert!(
        outcome
            .arms
            .iter()
            .any(|arm| arm.status == ArmStatus::Deferred),
        "exhausted capacity must defer leftover arms: {:?}",
        outcome.arms
    );
    assert_eq!(
        outcome.arms.iter().map(|arm| arm.byte_count).sum::<u64>(),
        surface
    );

    let mut bytes = poll_cycle_request(&set);
    bytes.message_id = IdToken::new("msg:aw-poll-req-bytes");
    bytes.bound = Some(PollBound {
        max_events: None,
        max_payload_refs: None,
        max_bytes: Some(surface),
    });
    let clock = FakeClock::auto(bytes.created_at.clone());
    let observer = EndlessReadyObserver::new();
    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        run_poll_cycle(&set, &bytes, &observer, &clock, &Cancel::new()),
    )
    .await
    .expect("byte bound must stop drain")
    .expect("poll cycle");
    admit(&outcome);
    assert_eq!(outcome.events.len(), 1);
    assert_eq!(outcome.outcome_kind, OutcomeKind::Partial);
    assert_eq!(
        outcome
            .arms
            .iter()
            .find(|arm| arm.status == ArmStatus::Events)
            .map(|arm| arm.byte_count),
        Some(surface)
    );
}

#[tokio::test]
async fn deferred_same_instant_event_replays_after_fairness_rotate() {
    let set = two_arm_set();
    let at = "2026-08-15T16:05:00Z";
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
    first.fairness_cursor = IdToken::new("fair:start");
    let clock = FakeClock::auto(first.created_at.clone());
    let observer = ScriptedObserver::new(script, clock.clone());
    let left = run_cycle(&set, &first, &observer, &clock).await;
    assert_eq!(left.outcome_kind, OutcomeKind::Partial);
    assert_eq!(
        left.events
            .iter()
            .map(|event| event.event_id.as_str())
            .collect::<Vec<_>>(),
        vec!["evt:left"]
    );
    assert_eq!(
        left.arms
            .iter()
            .find(|arm| arm.registration_id.as_str() == "reg:sms-1")
            .map(|arm| arm.status),
        Some(ArmStatus::Deferred)
    );

    let mut second = first.clone();
    second.message_id = IdToken::new("msg:aw-poll-req-2");
    second.fairness_cursor = left.next_fairness_cursor.clone();
    assert_ne!(
        second.fairness_cursor.as_str(),
        first.fairness_cursor.as_str()
    );
    let right = run_cycle(&set, &second, &observer, &clock).await;
    assert_eq!(
        right
            .events
            .iter()
            .map(|event| event.event_id.as_str())
            .collect::<Vec<_>>(),
        vec!["evt:right"],
        "deferred same-instant event must restore for the rotated cycle: {:?}",
        right.events
    );
    assert_eq!(right.outcome_kind, OutcomeKind::Partial);
    assert_eq!(observer.live_bind_count(), 0);
}

#[tokio::test]
async fn oversized_first_event_is_restored_not_dropped() {
    let set = registration_set(vec![registration(
        "reg:sms-1",
        "sms_inbound",
        "sms:inbox-1",
    )]);
    let sample = wait_event(
        "reg:sms-1",
        "sms_inbound",
        "evt:oversized",
        "2026-08-15T16:05:00Z",
    );
    let surface = event_surface_bytes(&sample);
    assert!(surface > 1);
    let script = Script {
        buffer_limit: 8,
        events: vec![sample],
    };
    let mut tight = poll_cycle_request(&set);
    tight.bound = Some(PollBound {
        max_events: None,
        max_payload_refs: None,
        max_bytes: Some(1),
    });
    let clock = FakeClock::auto(tight.created_at.clone());
    let observer = ScriptedObserver::new(script, clock.clone());
    let first = run_cycle(&set, &tight, &observer, &clock).await;
    assert!(
        first.events.is_empty(),
        "oversized event must not be kept: {:?}",
        first.events
    );
    assert_ne!(first.outcome_kind, OutcomeKind::Events);
    assert!(!first.coverage_complete);

    let mut room = tight.clone();
    room.message_id = IdToken::new("msg:aw-poll-req-2");
    room.bound = Some(PollBound {
        max_events: None,
        max_payload_refs: None,
        max_bytes: Some(surface),
    });
    let second = run_cycle(&set, &room, &observer, &clock).await;
    assert_eq!(
        second
            .events
            .iter()
            .map(|event| event.event_id.as_str())
            .collect::<Vec<_>>(),
        vec!["evt:oversized"],
        "rejected first event must restore for a later cycle: {:?}",
        second.events
    );
    assert_eq!(observer.live_bind_count(), 0);
}

#[tokio::test]
async fn admitted_events_and_cursors_commit_only_on_poll_cycle_ack() {
    assert!(
        POLL_ACK_RETENTION.contains("not committed until poll_cycle_ack"),
        "addendum must name the commit: {POLL_ACK_RETENTION}"
    );
    assert_eq!(MessageType::ALL.len(), 6);
    assert_eq!(
        MessageType::parse("poll_cycle_ack"),
        Some(MessageType::PollCycleAck)
    );
    assert_eq!(MessageType::parse("live_wait_ack"), None);

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
    let first_observer = ScriptedObserver::new(script.clone(), clock.clone());
    let first = run_cycle(&set, &request, &first_observer, &clock).await;
    assert_eq!(first.outcome_kind, OutcomeKind::Events);
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
    assert_ne!(proposed, start, "outcome may propose a later cursor");
    assert_eq!(
        first.retained_through.get("reg:sms-1"),
        Some(&proposed),
        "retained_through is a proposal until ack"
    );

    let clock = FakeClock::auto(request.created_at.clone());
    let replay_observer = ScriptedObserver::new(script, clock.clone());
    let replay = run_cycle(&set, &request, &replay_observer, &clock).await;
    assert_eq!(replay.events[0].event_id.as_str(), "evt:sms-1");
    assert_eq!(
        replay
            .arms
            .iter()
            .find(|arm| arm.registration_id.as_str() == "reg:sms-1")
            .map(|arm| &arm.start_anchor),
        Some(&start)
    );
    let first_json = serde_json::to_string(&AgentWaitMessage::PollCycleOutcome(first.clone()))
        .expect("serialize first");
    let replay_json = serde_json::to_string(&AgentWaitMessage::PollCycleOutcome(replay.clone()))
        .expect("serialize replay");
    validate_raw_documents([&first_json, &replay_json])
        .expect("unacked replay must not look like a silent advance");

    let ack = ack_poll_outcome(&first);
    admit_pair(&first, &ack);
    let mut committed_req = request.clone();
    committed_req.message_id = IdToken::new("msg:aw-poll-req-2");
    committed_req.acknowledged_anchors = ack.committed_anchors.clone();
    let clock = FakeClock::auto(committed_req.created_at.clone());
    let committed_observer = ScriptedObserver::new(Script::default(), clock.clone());
    let committed = run_cycle(&set, &committed_req, &committed_observer, &clock).await;
    assert!(committed.events.is_empty());
    assert_eq!(
        committed_observer.bind_requested_starts().get("reg:sms-1"),
        Some(&Some(proposed.clone()))
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

#[tokio::test]
async fn deferred_observations_replay_in_order_when_restored() {
    assert!(
        POLL_ACK_RETENTION.contains("deferred observations replay in order if restored"),
        "addendum must name restore order: {POLL_ACK_RETENTION}"
    );
    let set = two_arm_set();
    let at = "2026-08-15T16:05:00Z";
    let mut left = wait_event("reg:chanvoy-1", "chanvoy_wait", "evt:left", at);
    left.proposed_next_anchor.value = IdToken::new("anc:after-left");
    let mut right = wait_event("reg:sms-1", "sms_inbound", "evt:right", at);
    right.proposed_next_anchor.value = IdToken::new("anc:after-right");
    let script = Script {
        buffer_limit: 8,
        events: vec![left, right],
    };
    let mut first_req = poll_cycle_request(&set);
    first_req.bound = Some(PollBound {
        max_events: Some(1),
        max_payload_refs: None,
        max_bytes: None,
    });
    let clock = FakeClock::auto(first_req.created_at.clone());
    let observer = ScriptedObserver::new(script, clock.clone());
    let first = run_cycle(&set, &first_req, &observer, &clock).await;
    assert_eq!(first.outcome_kind, OutcomeKind::Partial);
    assert_eq!(
        first
            .events
            .iter()
            .map(|event| event.event_id.as_str())
            .collect::<Vec<_>>(),
        vec!["evt:left"]
    );
    assert_eq!(
        first
            .arms
            .iter()
            .find(|arm| arm.registration_id.as_str() == "reg:sms-1")
            .map(|arm| arm.status),
        Some(ArmStatus::Deferred)
    );
    assert_eq!(
        observer
            .queued_event_ids()
            .get("reg:sms-1")
            .map(Vec::as_slice),
        Some(["evt:right".to_string()].as_slice()),
        "deferred same-instant loser must be restored in order: {:?}",
        observer.queued_event_ids()
    );

    let mut second_req = first_req.clone();
    second_req.message_id = IdToken::new("msg:aw-poll-req-2");
    second_req.fairness_cursor = first.next_fairness_cursor.clone();
    let second = run_cycle(&set, &second_req, &observer, &clock).await;
    assert_eq!(
        second
            .events
            .iter()
            .map(|event| event.event_id.as_str())
            .collect::<Vec<_>>(),
        vec!["evt:right"],
        "restored deferred observation must replay next: {:?}",
        second.events
    );
    assert_eq!(
        second
            .proposed_next_anchors
            .get("reg:sms-1")
            .map(|a| a.value.as_str()),
        Some("anc:after-right")
    );
    assert_eq!(observer.live_bind_count(), 0);

    let mut one = registration("reg:sms-1", "sms_inbound", "sms:inbox-1");
    one.bounds.max_events = 1;
    let one_set = registration_set(vec![one]);
    let mut early = wait_event(
        "reg:sms-1",
        "sms_inbound",
        "evt:sms-1",
        "2026-08-15T16:05:00Z",
    );
    early.proposed_next_anchor.value = IdToken::new("anc:after-1");
    let mut late = wait_event(
        "reg:sms-1",
        "sms_inbound",
        "evt:sms-2",
        "2026-08-15T16:05:00Z",
    );
    late.proposed_next_anchor.value = IdToken::new("anc:after-2");
    let ordered = Script {
        buffer_limit: 8,
        events: vec![early, late],
    };
    let one_req = poll_cycle_request(&one_set);
    let clock = FakeClock::auto(one_req.created_at.clone());
    let observer = ScriptedObserver::new(ordered, clock.clone());
    let first = run_cycle(&one_set, &one_req, &observer, &clock).await;
    assert_eq!(
        first
            .events
            .iter()
            .map(|event| event.event_id.as_str())
            .collect::<Vec<_>>(),
        vec!["evt:sms-1"]
    );
    assert_eq!(
        observer
            .queued_event_ids()
            .get("reg:sms-1")
            .map(Vec::as_slice),
        Some(["evt:sms-1".to_string(), "evt:sms-2".to_string()].as_slice()),
        "admitted head and leftover must stay queued in order: {:?}",
        observer.queued_event_ids()
    );
    let mut second_req = one_req.clone();
    second_req.message_id = IdToken::new("msg:aw-poll-req-2");
    let second = run_cycle(&one_set, &second_req, &observer, &clock).await;
    assert_eq!(
        second
            .events
            .iter()
            .map(|event| event.event_id.as_str())
            .collect::<Vec<_>>(),
        vec!["evt:sms-1"],
        "unacked same-observer restart must replay the admitted head: {:?}",
        second.events
    );
    assert_eq!(
        observer
            .queued_event_ids()
            .get("reg:sms-1")
            .map(Vec::as_slice),
        Some(["evt:sms-1".to_string(), "evt:sms-2".to_string()].as_slice())
    );
}

#[tokio::test]
async fn cancel_bound_exhaustion_and_restart_before_ack_do_not_advance_cursors() {
    assert!(
        POLL_ACK_RETENTION.contains("must not silently advance cursors"),
        "addendum must name silent-advance: {POLL_ACK_RETENTION}"
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
            "2026-08-15T16:05:00Z",
        )],
    };
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = ScriptedObserver::new(script, clock.clone());
    let first = run_cycle(&set, &request, &observer, &clock).await;
    assert_eq!(first.outcome_kind, OutcomeKind::Events);
    assert_eq!(
        observer
            .queued_event_ids()
            .get("reg:sms-1")
            .map(Vec::as_slice),
        Some(["evt:sms-1".to_string()].as_slice()),
        "admitted event must stay queued on the same observer until ack"
    );
    let original = Anchor {
        kind: AnchorKind::ProviderOpaque,
        value: IdToken::new("anc:cursor-0"),
    };
    let proposed = first
        .proposed_next_anchors
        .get("reg:sms-1")
        .expect("proposed")
        .clone();
    assert_ne!(proposed, original);

    let cancel = Cancel::new();
    cancel.trigger();
    let cancelled = run_poll_cycle(&set, &request, &observer, &clock, &cancel)
        .await
        .expect("pre-ack cancel");
    admit(&cancelled);
    assert_eq!(cancelled.outcome_kind, OutcomeKind::Cancelled);
    assert_eq!(
        cancelled.retained_through.get("reg:sms-1"),
        Some(&original),
        "cancel before ack must keep the start cursor"
    );
    assert_eq!(
        cancelled.proposed_next_anchors.get("reg:sms-1"),
        Some(&original)
    );
    assert_ne!(
        cancelled.retained_through.get("reg:sms-1"),
        Some(&proposed),
        "cancel must not inherit an unacked proposed cursor"
    );
    assert_eq!(
        observer
            .queued_event_ids()
            .get("reg:sms-1")
            .map(Vec::as_slice),
        Some(["evt:sms-1".to_string()].as_slice()),
        "cancel after an unacked outcome must not drop the same-observer event"
    );

    let mut tight_reg = registration("reg:sms-1", "sms_inbound", "sms:inbox-1");
    tight_reg.bounds.max_events = 1;
    let tight_set = registration_set(vec![tight_reg]);
    let mut first_evt = wait_event(
        "reg:sms-1",
        "sms_inbound",
        "evt:sms-1",
        "2026-08-15T16:05:00Z",
    );
    first_evt.proposed_next_anchor.value = IdToken::new("anc:after-1");
    let mut leftover = wait_event(
        "reg:sms-1",
        "sms_inbound",
        "evt:sms-2",
        "2026-08-15T16:05:00Z",
    );
    leftover.proposed_next_anchor.value = IdToken::new("anc:after-2");
    let bounded = Script {
        buffer_limit: 8,
        events: vec![first_evt, leftover],
    };
    let tight_req = poll_cycle_request(&tight_set);
    let clock = FakeClock::auto(tight_req.created_at.clone());
    let observer = ScriptedObserver::new(bounded.clone(), clock.clone());
    let exhausted = run_cycle(&tight_set, &tight_req, &observer, &clock).await;
    assert_eq!(exhausted.outcome_kind, OutcomeKind::Partial);
    assert_eq!(exhausted.events[0].event_id.as_str(), "evt:sms-1");
    assert_eq!(
        exhausted
            .retained_through
            .get("reg:sms-1")
            .map(|a| a.value.as_str()),
        Some("anc:after-1")
    );
    assert_eq!(
        observer
            .queued_event_ids()
            .get("reg:sms-1")
            .map(Vec::as_slice),
        Some(["evt:sms-1".to_string(), "evt:sms-2".to_string()].as_slice()),
        "bound exhaustion must restore admitted and leftover on the same observer"
    );

    let restart = run_cycle(&tight_set, &tight_req, &observer, &clock).await;
    assert_eq!(restart.events[0].event_id.as_str(), "evt:sms-1");
    assert_eq!(
        restart
            .arms
            .iter()
            .find(|arm| arm.registration_id.as_str() == "reg:sms-1")
            .map(|arm| arm.start_anchor.value.as_str()),
        Some("anc:cursor-0"),
        "restart between outcome and ack must bind the original start"
    );
    let exhausted_json =
        serde_json::to_string(&AgentWaitMessage::PollCycleOutcome(exhausted)).expect("serialize");
    let restart_json =
        serde_json::to_string(&AgentWaitMessage::PollCycleOutcome(restart)).expect("serialize");
    validate_raw_documents([&exhausted_json, &restart_json])
        .expect("unacked bound-exhaustion restart must not silently advance");
}

#[tokio::test]
async fn same_observer_keeps_admitted_events_until_ack() {
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
    let observer = ScriptedObserver::new(script, clock.clone());
    let first = run_cycle(&set, &request, &observer, &clock).await;
    assert_eq!(first.outcome_kind, OutcomeKind::Events);
    assert_eq!(first.events[0].event_id.as_str(), "evt:sms-1");
    assert_eq!(
        observer
            .queued_event_ids()
            .get("reg:sms-1")
            .map(Vec::as_slice),
        Some(["evt:sms-1".to_string()].as_slice()),
        "same observer must still hold the admitted event before ack"
    );
    assert_eq!(
        first
            .arms
            .iter()
            .find(|arm| arm.registration_id.as_str() == "reg:sms-1")
            .map(|arm| arm.start_anchor.value.as_str()),
        Some("anc:cursor-0")
    );

    let mut second_req = request.clone();
    second_req.message_id = IdToken::new("msg:aw-poll-req-2");
    let second = run_cycle(&set, &second_req, &observer, &clock).await;
    assert_eq!(second.events[0].event_id.as_str(), "evt:sms-1");
    assert_eq!(
        second
            .arms
            .iter()
            .find(|arm| arm.registration_id.as_str() == "reg:sms-1")
            .map(|arm| arm.start_anchor.value.as_str()),
        Some("anc:cursor-0"),
        "restart before ack must not silently advance the bind start"
    );
    let first_json =
        serde_json::to_string(&AgentWaitMessage::PollCycleOutcome(first)).expect("serialize first");
    let second_json = serde_json::to_string(&AgentWaitMessage::PollCycleOutcome(second))
        .expect("serialize second");
    validate_raw_documents([&first_json, &second_json])
        .expect("same-observer unacked replay must not look like silent advance");
}

#[tokio::test]
async fn cancel_after_record_ready_restores_on_same_observer() {
    let set = three_arm_set();
    let request = poll_cycle_request(&set);
    let script = Script {
        buffer_limit: 8,
        events: vec![
            wait_event(
                "reg:chanvoy-1",
                "chanvoy_wait",
                "evt:chanvoy-1",
                "2026-08-15T16:05:00Z",
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
    observer.hang_bind("reg:job-1");
    let cancel = Cancel::new();
    let set2 = set.clone();
    let request2 = request.clone();
    let clock2 = clock.clone();
    let observer2 = observer.clone();
    let cancel2 = cancel.clone();
    let task = tokio::spawn(async move {
        run_poll_cycle(&set2, &request2, &observer2, &clock2, &cancel2).await
    });
    while observer
        .queued_event_ids()
        .get("reg:chanvoy-1")
        .is_none_or(|ids| !ids.is_empty())
        || observer
            .queued_event_ids()
            .get("reg:sms-1")
            .is_none_or(|ids| !ids.is_empty())
    {
        tokio::task::yield_now().await;
    }
    cancel.trigger();
    let outcome = task.await.expect("join").expect("cancelled");
    admit(&outcome);
    assert_eq!(outcome.outcome_kind, OutcomeKind::Cancelled);
    assert_eq!(
        observer
            .queued_event_ids()
            .get("reg:chanvoy-1")
            .map(Vec::as_slice),
        Some(["evt:chanvoy-1".to_string()].as_slice()),
        "cancel after record_ready must restore admitted events: {:?}",
        observer.queued_event_ids()
    );
    assert_eq!(
        observer
            .queued_event_ids()
            .get("reg:sms-1")
            .map(Vec::as_slice),
        Some(["evt:sms-1".to_string()].as_slice())
    );
    for rid in ["reg:chanvoy-1", "reg:sms-1", "reg:job-1"] {
        if let Some(cursor) = outcome.retained_through.get(rid) {
            assert_eq!(
                cursor.value.as_str(),
                "anc:cursor-0",
                "cancel must not advance {rid}"
            );
        }
    }

    observer.release_hang_bind("reg:job-1");
    let replay = run_cycle(&set, &request, &observer, &clock).await;
    assert!(
        replay
            .events
            .iter()
            .any(|event| event.event_id.as_str() == "evt:chanvoy-1"),
        "same observer must replay restored events after cancel: {:?}",
        replay.events
    );
}

#[test]
fn maps_stay_keyed_by_registration_id() {
    let mut retained_through = BTreeMap::new();
    retained_through.insert(
        "reg:sms-1".to_string(),
        Anchor {
            kind: AnchorKind::ProviderOpaque,
            value: IdToken::new("anc:cursor-0"),
        },
    );
    assert!(retained_through.contains_key("reg:sms-1"));
    assert!(!retained_through.contains_key("arm:sms-1"));
}
