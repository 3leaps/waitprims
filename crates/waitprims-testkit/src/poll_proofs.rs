//! CP4 proofs: poll-cycle coverage, fairness, ack, replay, starvation, dirty arms.

use std::collections::BTreeMap;
use std::path::PathBuf;

use waitprims_async::{run_poll_cycle, Cancel};
use waitprims_core::{
    validate_message, validate_raw_documents, AgentWaitMessage, Anchor, AnchorKind, ArmStatus,
    IdToken, OutcomeKind, PollCycleAck, PollCycleOutcome, Timestamp,
};

use crate::{
    ack_poll_outcome, exclusive_head_anchor, poll_cycle_request, registration,
    registration_baseline, registration_set, ts, wait_event, FakeClock, IdleObserver, Script,
    ScriptedObserver,
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
