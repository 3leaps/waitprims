//! Proofs: held-session emit policy (`run_coalesce`).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use waitprims_async::{
    run_coalesce, run_follow, Cancel, CoalescePolicy, FollowEnd, Observation, Observer,
    TerminalArmKind,
};
use waitprims_core::{Registration, Result, ValidationError, PRIORITY_URGENT};

use crate::{
    live_wait_request, registration, registration_set, ts, wait_event, with_priority, FakeClock,
    Script, ScriptedObserver, TrackedBind,
};

fn ids(bursts: &Mutex<Vec<Vec<String>>>) -> Vec<Vec<String>> {
    bursts.lock().expect("bursts").clone()
}

fn push_ids(bursts: &Arc<Mutex<Vec<Vec<String>>>>, burst: &waitprims_async::CoalesceBurst) {
    bursts.lock().expect("bursts").push(
        burst
            .events
            .iter()
            .map(|event| event.event_id.as_str().to_string())
            .collect(),
    );
}

#[tokio::test]
async fn timer_flush_without_later_observation() {
    let set = registration_set(vec![registration(
        "reg:sms-1",
        "sms_inbound",
        "sms:inbox-1",
    )]);
    let request = live_wait_request();
    let script = Script {
        buffer_limit: 8,
        events: vec![wait_event(
            "reg:sms-1",
            "sms_inbound",
            "evt:1",
            "2026-08-15T16:05:00Z",
        )],
    };
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = ScriptedObserver::new(script, clock.clone());
    let bursts = Arc::new(Mutex::new(Vec::new()));
    let policy = CoalescePolicy::new(Duration::from_secs(10));
    let end = run_coalesce(
        &observer,
        &clock,
        &Cancel::new(),
        &set,
        &request,
        &policy,
        {
            let bursts = bursts.clone();
            move |burst| {
                push_ids(&bursts, &burst);
                async { Ok(()) }
            }
        },
    )
    .await
    .expect("coalesce");
    assert_eq!(end, FollowEnd::Deadline);
    assert_eq!(ids(&bursts), vec![vec!["evt:1".to_string()]]);
    assert_eq!(observer.live_bind_count(), 0);
}

#[tokio::test]
async fn urgent_emits_immediately_quiet_waits() {
    let set = registration_set(vec![
        registration("reg:quiet", "sms_inbound", "sms:q"),
        with_priority(
            registration("reg:urgent", "sms_inbound", "sms:u"),
            PRIORITY_URGENT,
        ),
    ]);
    let request = live_wait_request();
    let at = "2026-08-15T16:05:00Z";
    let script = Script {
        buffer_limit: 8,
        events: vec![
            wait_event("reg:quiet", "sms_inbound", "evt:quiet", at),
            wait_event("reg:urgent", "sms_inbound", "evt:urgent", at),
        ],
    };
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = ScriptedObserver::new(script, clock.clone());
    let bursts = Arc::new(Mutex::new(Vec::new()));
    let policy = CoalescePolicy::new(Duration::from_secs(10));
    let end = run_coalesce(
        &observer,
        &clock,
        &Cancel::new(),
        &set,
        &request,
        &policy,
        {
            let bursts = bursts.clone();
            move |burst| {
                push_ids(&bursts, &burst);
                async { Ok(()) }
            }
        },
    )
    .await
    .expect("coalesce");
    assert_eq!(end, FollowEnd::Deadline);
    assert_eq!(
        ids(&bursts),
        vec![
            vec!["evt:urgent".to_string()],
            vec!["evt:quiet".to_string()]
        ]
    );
}

#[tokio::test]
async fn hitchhike_when_interval_due() {
    let set = registration_set(vec![
        registration("reg:quiet", "sms_inbound", "sms:q"),
        with_priority(
            registration("reg:urgent", "sms_inbound", "sms:u"),
            PRIORITY_URGENT,
        ),
    ]);
    let request = live_wait_request();
    let script = Script {
        buffer_limit: 8,
        events: vec![
            wait_event(
                "reg:quiet",
                "sms_inbound",
                "evt:quiet",
                "2026-08-15T16:05:00Z",
            ),
            wait_event(
                "reg:urgent",
                "sms_inbound",
                "evt:urgent",
                "2026-08-15T16:15:00Z",
            ),
        ],
    };
    let clock = FakeClock::manual(request.created_at.clone());
    let observer = ScriptedObserver::new(script, clock.clone());
    let bursts = Arc::new(Mutex::new(Vec::new()));
    let policy = CoalescePolicy::new(Duration::from_secs(10));
    let cancel = Cancel::new();
    async fn pump(clock: &FakeClock, at: &str) {
        clock.advance_to(&ts(at));
        for _ in 0..64 {
            tokio::task::yield_now().await;
        }
    }
    let (end, _) = tokio::join!(
        run_coalesce(&observer, &clock, &cancel, &set, &request, &policy, {
            let bursts = bursts.clone();
            move |burst| {
                push_ids(&bursts, &burst);
                async { Ok(()) }
            }
        },),
        async {
            pump(&clock, "2026-08-15T16:05:00Z").await;
            pump(&clock, "2026-08-15T16:15:00Z").await;
            pump(&clock, "2026-08-15T16:20:00Z").await;
        }
    );
    let end = end.expect("coalesce");
    assert_eq!(end, FollowEnd::Deadline);
    assert_eq!(
        ids(&bursts),
        vec![vec!["evt:quiet".to_string(), "evt:urgent".to_string()]]
    );
}

#[tokio::test]
async fn multi_turn_quiet_preserves_fifo() {
    let set = registration_set(vec![registration(
        "reg:sms-1",
        "sms_inbound",
        "sms:inbox-1",
    )]);
    let request = live_wait_request();
    let script = Script {
        buffer_limit: 8,
        events: vec![
            wait_event("reg:sms-1", "sms_inbound", "evt:1", "2026-08-15T16:05:00Z"),
            wait_event("reg:sms-1", "sms_inbound", "evt:2", "2026-08-15T16:06:00Z"),
        ],
    };
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = ScriptedObserver::new(script, clock.clone());
    let bursts = Arc::new(Mutex::new(Vec::new()));
    let policy = CoalescePolicy::new(Duration::from_secs(3600));
    let end = run_coalesce(
        &observer,
        &clock,
        &Cancel::new(),
        &set,
        &request,
        &policy,
        {
            let bursts = bursts.clone();
            move |burst| {
                push_ids(&bursts, &burst);
                async { Ok(()) }
            }
        },
    )
    .await
    .expect("coalesce");
    assert_eq!(end, FollowEnd::Deadline);
    assert_eq!(
        ids(&bursts),
        vec![vec!["evt:1".to_string(), "evt:2".to_string()]]
    );
}

#[tokio::test]
async fn pending_overflow_is_fail_closed() {
    let mut set = registration_set(vec![registration(
        "reg:sms-1",
        "sms_inbound",
        "sms:inbox-1",
    )]);
    set.aggregate_limits.max_events = 1;
    let request = live_wait_request();
    let script = Script {
        buffer_limit: 8,
        events: vec![
            wait_event("reg:sms-1", "sms_inbound", "evt:1", "2026-08-15T16:05:00Z"),
            wait_event("reg:sms-1", "sms_inbound", "evt:2", "2026-08-15T16:06:00Z"),
        ],
    };
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = ScriptedObserver::new(script, clock.clone());
    let policy = CoalescePolicy::new(Duration::from_secs(3600));
    let end = run_coalesce(
        &observer,
        &clock,
        &Cancel::new(),
        &set,
        &request,
        &policy,
        |_| async { Ok(()) },
    )
    .await
    .expect("coalesce");
    assert_eq!(
        end,
        FollowEnd::TerminalArm {
            registration_id: waitprims_core::IdToken::new("reg:sms-1"),
            kind: TerminalArmKind::Overflow,
            reason_code: waitprims_core::IdToken::new("buffer_overflow"),
        }
    );
    assert_eq!(observer.live_bind_count(), 0);
}

#[tokio::test]
async fn terminal_flush_is_ungated() {
    let set = registration_set(vec![registration(
        "reg:sms-1",
        "sms_inbound",
        "sms:inbox-1",
    )]);
    let request = live_wait_request();
    let script = Script {
        buffer_limit: 8,
        events: vec![wait_event(
            "reg:sms-1",
            "sms_inbound",
            "evt:held",
            "2026-08-15T16:05:00Z",
        )],
    };
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = ScriptedObserver::new(script, clock.clone());
    let bursts = Arc::new(Mutex::new(Vec::new()));
    let policy = CoalescePolicy::new(Duration::from_secs(3600));
    let end = run_coalesce(
        &observer,
        &clock,
        &Cancel::new(),
        &set,
        &request,
        &policy,
        {
            let bursts = bursts.clone();
            move |burst| {
                push_ids(&bursts, &burst);
                async { Ok(()) }
            }
        },
    )
    .await
    .expect("coalesce");
    assert_eq!(end, FollowEnd::Deadline);
    assert_eq!(ids(&bursts), vec![vec!["evt:held".to_string()]]);
}

#[tokio::test]
async fn sink_error_drops_buffer() {
    let set = registration_set(vec![with_priority(
        registration("reg:sms-1", "sms_inbound", "sms:inbox-1"),
        PRIORITY_URGENT,
    )]);
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
    let policy = CoalescePolicy::new(Duration::from_secs(10));
    let err = run_coalesce(
        &observer,
        &clock,
        &Cancel::new(),
        &set,
        &request,
        &policy,
        |_| async { Err(ValidationError::new("/sink", "refused").into()) },
    )
    .await
    .expect_err("sink err");
    assert!(err.to_string().contains("refused"), "{err}");
    assert_eq!(observer.live_bind_count(), 0);
}

#[tokio::test]
async fn future_drop_releases_binds_without_flush() {
    let set = registration_set(vec![with_priority(
        registration("reg:sms-1", "sms_inbound", "sms:inbox-1"),
        PRIORITY_URGENT,
    )]);
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
    let started = Arc::new(Mutex::new(false));
    let policy = CoalescePolicy::new(Duration::from_secs(10));
    let cancel = Cancel::new();
    tokio::select! {
        biased;
        result = run_coalesce(&observer, &clock, &cancel, &set, &request, &policy, {
            let started = started.clone();
            move |_| {
                *started.lock().expect("started") = true;
                async {
                    std::future::pending::<()>().await;
                    Ok(())
                }
            }
        }) => panic!("dropped coalesce must not complete: {result:?}"),
        _ = async {
            while !*started.lock().expect("started") {
                tokio::task::yield_now().await;
            }
        } => {}
    }
    assert_eq!(observer.live_bind_count(), 0);
}

#[tokio::test]
async fn omitted_priority_reads_as_normal() {
    let mut policy = CoalescePolicy::new(Duration::from_secs(10));
    policy.urgent_at = 50;
    let set = registration_set(vec![registration(
        "reg:sms-1",
        "sms_inbound",
        "sms:inbox-1",
    )]);
    let request = live_wait_request();
    let script = Script {
        buffer_limit: 8,
        events: vec![wait_event(
            "reg:sms-1",
            "sms_inbound",
            "evt:1",
            "2026-08-15T16:05:00Z",
        )],
    };
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = ScriptedObserver::new(script, clock.clone());
    let bursts = Arc::new(Mutex::new(Vec::new()));
    let cancel = Cancel::new();
    let end = run_coalesce(&observer, &clock, &cancel, &set, &request, &policy, {
        let bursts = bursts.clone();
        let cancel = cancel.clone();
        move |burst| {
            push_ids(&bursts, &burst);
            cancel.trigger();
            async { Ok(()) }
        }
    })
    .await
    .expect("coalesce");
    assert_eq!(end, FollowEnd::Cancel);
    assert_eq!(ids(&bursts), vec![vec!["evt:1".to_string()]]);
}

#[tokio::test]
async fn follow_ignores_priority() {
    let set = registration_set(vec![
        with_priority(registration("reg:chanvoy-1", "chanvoy_wait", "chan:a"), 0),
        with_priority(registration("reg:sms-1", "sms_inbound", "sms:inbox-1"), 255),
    ]);
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
    let bursts = Arc::new(Mutex::new(Vec::new()));
    let end = run_follow(&observer, &clock, &Cancel::new(), &set, &request, {
        let bursts = bursts.clone();
        move |burst| {
            bursts.lock().expect("bursts").push(
                burst
                    .events
                    .iter()
                    .map(|event| event.event_id.as_str().to_string())
                    .collect::<Vec<_>>(),
            );
            async { Ok(()) }
        }
    })
    .await
    .expect("follow");
    assert_eq!(end, FollowEnd::Deadline);
    assert_eq!(
        bursts.lock().expect("bursts").clone(),
        vec![vec!["evt:chanvoy-1".to_string(), "evt:sms-1".to_string()]]
    );
}

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

    fn restore_ready(&self, bind: &Self::Bind, obs: Observation) -> Result<()> {
        self.inner.restore_ready(bind, obs)
    }
}

#[tokio::test]
async fn delayed_arrival_uses_clock_now() {
    let set = registration_set(vec![registration(
        "reg:sms-1",
        "sms_inbound",
        "sms:inbox-1",
    )]);
    let request = live_wait_request();
    let script = Script {
        buffer_limit: 8,
        events: vec![wait_event(
            "reg:sms-1",
            "sms_inbound",
            "evt:past",
            "2026-08-15T15:00:00Z",
        )],
    };
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = ScriptedObserver::new(script, clock.clone());
    let emit_at = Arc::new(Mutex::new(Vec::new()));
    let policy = CoalescePolicy::new(Duration::from_secs(10));
    let end = run_coalesce(
        &observer,
        &clock,
        &Cancel::new(),
        &set,
        &request,
        &policy,
        {
            let emit_at = emit_at.clone();
            let clock = clock.clone();
            move |burst| {
                assert_eq!(burst.events[0].event_id.as_str(), "evt:past");
                emit_at
                    .lock()
                    .expect("times")
                    .push(clock.current_time().as_str().to_string());
                async { Ok(()) }
            }
        },
    )
    .await
    .expect("coalesce");
    assert_eq!(end, FollowEnd::Deadline);
    assert_eq!(
        emit_at.lock().expect("times").clone(),
        vec!["2026-08-15T16:01:10Z".to_string()]
    );
}

#[tokio::test]
async fn consecutive_windows_advance_cadence() {
    let set = registration_set(vec![registration(
        "reg:sms-1",
        "sms_inbound",
        "sms:inbox-1",
    )]);
    let mut request = live_wait_request();
    request.run_deadline = ts("2026-08-15T16:40:00Z");
    let script = Script {
        buffer_limit: 8,
        events: vec![
            wait_event("reg:sms-1", "sms_inbound", "evt:1", "2026-08-15T16:05:00Z"),
            wait_event("reg:sms-1", "sms_inbound", "evt:2", "2026-08-15T16:16:00Z"),
        ],
    };
    let clock = FakeClock::manual(request.created_at.clone());
    let observer = ScriptedObserver::new(script, clock.clone());
    let emit_at = Arc::new(Mutex::new(Vec::new()));
    let policy = CoalescePolicy::new(Duration::from_secs(600));
    let cancel = Cancel::new();
    async fn pump(clock: &FakeClock, at: &str) {
        clock.advance_to(&ts(at));
        for _ in 0..64 {
            tokio::task::yield_now().await;
        }
    }
    let (end, _) = tokio::join!(
        run_coalesce(&observer, &clock, &cancel, &set, &request, &policy, {
            let emit_at = emit_at.clone();
            let clock = clock.clone();
            move |burst| {
                emit_at.lock().expect("times").push((
                    clock.current_time().as_str().to_string(),
                    burst
                        .events
                        .iter()
                        .map(|event| event.event_id.as_str().to_string())
                        .collect::<Vec<_>>(),
                ));
                async { Ok(()) }
            }
        },),
        async {
            pump(&clock, "2026-08-15T16:05:00Z").await;
            pump(&clock, "2026-08-15T16:15:00Z").await;
            while emit_at.lock().expect("times").is_empty() {
                tokio::task::yield_now().await;
            }
            pump(&clock, "2026-08-15T16:16:00Z").await;
            pump(&clock, "2026-08-15T16:25:00Z").await;
            pump(&clock, "2026-08-15T16:40:00Z").await;
        }
    );
    let end = end.expect("coalesce");
    assert_eq!(end, FollowEnd::Deadline);
    assert_eq!(
        emit_at.lock().expect("times").clone(),
        vec![
            (
                "2026-08-15T16:15:00Z".to_string(),
                vec!["evt:1".to_string()]
            ),
            (
                "2026-08-15T16:25:00Z".to_string(),
                vec!["evt:2".to_string()]
            )
        ]
    );
}

#[tokio::test]
async fn sibling_event_plus_fault_restores() {
    let set = registration_set(vec![
        registration("reg:chanvoy-1", "chanvoy_wait", "chan:seat-a"),
        registration("reg:sms-1", "sms_inbound", "sms:inbox-1"),
    ]);
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
    let policy = CoalescePolicy::new(Duration::from_secs(10));
    let err = run_coalesce(
        &observer,
        &clock,
        &Cancel::new(),
        &set,
        &request,
        &policy,
        |_| async { Ok(()) },
    )
    .await
    .expect_err("arm error");
    assert!(err.to_string().contains("injected_arm_error"), "{err}");
    assert_eq!(
        inner
            .queued_event_ids()
            .get("reg:chanvoy-1")
            .map(Vec::as_slice),
        Some(["evt:chanvoy-1".to_string()].as_slice()),
        "sibling event must be restored: {:?}",
        inner.queued_event_ids()
    );
    assert_eq!(inner.live_bind_count(), 0);
}

#[tokio::test]
async fn buffered_quiet_then_posture_err_restores() {
    let mut leased = registration("reg:sms-1", "sms_inbound", "sms:inbox-1");
    leased.lease_expires_at = ts("2026-08-15T16:06:00Z");
    let set = registration_set(vec![leased]);
    let request = live_wait_request();
    let script = Script {
        buffer_limit: 8,
        events: vec![wait_event(
            "reg:sms-1",
            "sms_inbound",
            "evt:held",
            "2026-08-15T16:05:00Z",
        )],
    };
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = ScriptedObserver::new(script, clock.clone());
    let policy = CoalescePolicy::new(Duration::from_secs(600));
    let err = run_coalesce(
        &observer,
        &clock,
        &Cancel::new(),
        &set,
        &request,
        &policy,
        |_| async { Ok(()) },
    )
    .await
    .expect_err("lease");
    assert!(err.to_string().contains("lease_expired"), "{err}");
    assert_eq!(
        observer
            .queued_event_ids()
            .get("reg:sms-1")
            .map(Vec::as_slice),
        Some(["evt:held".to_string()].as_slice()),
        "buffered quiet must be restored on posture err: {:?}",
        observer.queued_event_ids()
    );
    assert_eq!(observer.live_bind_count(), 0);
}
