//! Proofs: held-session emit policy (`run_coalesce`).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use waitprims_async::{
    run_coalesce, run_follow, BindHandle, Cancel, CoalescePolicy, FollowEnd, Observation, Observer,
    TerminalArmKind,
};
use waitprims_core::{NormativeReason, Registration, Result, ValidationError, PRIORITY_URGENT};

use crate::{
    live_wait_request, registration, registration_set, ts, wait_event, with_priority, FakeClock,
    IdleObserver, Script, ScriptedObserver, TrackedBind,
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
    assert_eq!(
        observer
            .queued_event_ids()
            .get("reg:sms-1")
            .map(Vec::as_slice),
        Some(["evt:1".to_string(), "evt:2".to_string()].as_slice()),
        "overflow must restore unsunk events: {:?}",
        observer.queued_event_ids()
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

fn lease_err(err: &waitprims_core::Error) -> &waitprims_core::ValidationError {
    match err {
        waitprims_core::Error::Validation(err) => err,
        other => panic!("expected validation, got {other}"),
    }
}

struct IdleAndScripted {
    idle_id: String,
    inner: ScriptedObserver,
    idle: IdleObserver,
}

impl Observer for IdleAndScripted {
    type Bind = TrackedBind;

    async fn bind(&self, registration: &Registration) -> Result<Self::Bind> {
        if registration.registration_id.as_str() == self.idle_id {
            self.idle.bind(registration).await
        } else {
            self.inner.bind(registration).await
        }
    }

    async fn next(&self, bind: &Self::Bind) -> Result<Observation> {
        if bind.registration_id.as_str() == self.idle_id {
            self.idle.next(bind).await
        } else {
            self.inner.next(bind).await
        }
    }

    async fn cancel(&self, bind: &Self::Bind) -> Result<()> {
        if bind.registration_id.as_str() == self.idle_id {
            self.idle.cancel(bind).await
        } else {
            self.inner.cancel(bind).await
        }
    }

    fn restore_ready(&self, bind: &Self::Bind, obs: Observation) -> Result<()> {
        if bind.registration_id.as_str() == self.idle_id {
            self.idle.restore_ready(bind, obs)
        } else {
            self.inner.restore_ready(bind, obs)
        }
    }
}

struct RestoreFailAlways {
    inner: ScriptedObserver,
    attempts: Arc<Mutex<Vec<String>>>,
}

impl Observer for RestoreFailAlways {
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
        self.attempts
            .lock()
            .expect("attempts")
            .push(bind.registration_id().as_str().to_string());
        let _ = obs;
        Err(ValidationError::new("/observer/restore_ready", "restore_failed").into())
    }
}

#[tokio::test]
async fn event_at_lease_restores_before_sink() {
    let mut leased = registration("reg:sms-1", "sms_inbound", "sms:inbox-1");
    leased.lease_expires_at = ts("2026-08-15T16:05:00Z");
    let set = registration_set(vec![leased]);
    let request = live_wait_request();
    let script = Script {
        buffer_limit: 8,
        events: vec![wait_event(
            "reg:sms-1",
            "sms_inbound",
            "evt:lease-1",
            "2026-08-15T16:05:00Z",
        )],
    };
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = ScriptedObserver::new(script, clock.clone());
    let bursts = Arc::new(Mutex::new(Vec::new()));
    let policy = CoalescePolicy::new(Duration::from_secs(10));
    let err = run_coalesce(
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
    .expect_err("lease");
    let err = lease_err(&err);
    assert_eq!(err.reason, Some(NormativeReason::LeaseReauth));
    assert!(ids(&bursts).is_empty());
    assert_eq!(
        observer
            .queued_event_ids()
            .get("reg:sms-1")
            .map(Vec::as_slice),
        Some(["evt:lease-1".to_string()].as_slice()),
        "event at lease must be restored: {:?}",
        observer.queued_event_ids()
    );
    assert_eq!(observer.live_bind_count(), 0);
}

#[tokio::test]
async fn fault_at_lease_is_posture() {
    let mut a = registration("reg:chanvoy-1", "chanvoy_wait", "chan:a");
    let mut b = registration("reg:sms-1", "sms_inbound", "sms:inbox-1");
    a.lease_expires_at = ts("2026-08-15T16:05:00Z");
    b.lease_expires_at = ts("2026-08-15T16:05:00Z");
    let set = registration_set(vec![a, b]);
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
    .expect_err("lease");
    let err = lease_err(&err);
    assert_eq!(err.reason, Some(NormativeReason::LeaseReauth));
    assert_eq!(inner.live_bind_count(), 0);
}

#[tokio::test]
async fn pending_and_harvested_same_arm_restore_fifo() {
    let mut leased = registration("reg:sms-1", "sms_inbound", "sms:inbox-1");
    leased.lease_expires_at = ts("2026-08-15T16:06:00Z");
    let set = registration_set(vec![leased]);
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
    let err = lease_err(&err);
    assert_eq!(err.reason, Some(NormativeReason::LeaseReauth));
    assert_eq!(
        observer
            .queued_event_ids()
            .get("reg:sms-1")
            .map(Vec::as_slice),
        Some(["evt:1".to_string(), "evt:2".to_string()].as_slice()),
        "FIFO A then B after restore: {:?}",
        observer.queued_event_ids()
    );
    assert_eq!(observer.live_bind_count(), 0);
}

#[tokio::test]
async fn restore_failure_attempts_every_observation() {
    let mut a = registration("reg:chanvoy-1", "chanvoy_wait", "chan:a");
    let mut b = registration("reg:sms-1", "sms_inbound", "sms:inbox-1");
    a.lease_expires_at = ts("2026-08-15T16:05:00Z");
    b.lease_expires_at = ts("2026-08-15T16:05:00Z");
    let set = registration_set(vec![a, b]);
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
    let attempts = Arc::new(Mutex::new(Vec::new()));
    let observer = RestoreFailAlways {
        inner: inner.clone(),
        attempts: attempts.clone(),
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
    .expect_err("restore");
    assert!(err.to_string().contains("restore_failed"), "{err}");
    assert_eq!(
        attempts.lock().expect("attempts").len(),
        2,
        "must attempt every restore: {:?}",
        attempts.lock().expect("attempts")
    );
    assert_eq!(inner.live_bind_count(), 0);
}

#[tokio::test]
async fn idle_backoff_does_not_delay_quiet_flush() {
    let set = registration_set(vec![
        registration("reg:sms-1", "sms_inbound", "sms:inbox-1"),
        registration("reg:idle-1", "sms_inbound", "sms:idle"),
    ]);
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
    let inner = ScriptedObserver::new(script, clock.clone());
    let observer = IdleAndScripted {
        idle_id: "reg:idle-1".to_string(),
        inner: inner.clone(),
        idle: IdleObserver::new(),
    };
    let emit_at = Arc::new(Mutex::new(Vec::new()));
    let policy = CoalescePolicy::new(Duration::from_secs(10));
    let _end = run_coalesce(
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
                emit_at
                    .lock()
                    .expect("times")
                    .push(clock.current_time().as_str().to_string());
                let _ = burst;
                async { Ok(()) }
            }
        },
    )
    .await
    .expect("coalesce");
    assert_eq!(
        emit_at.lock().expect("times").clone(),
        vec!["2026-08-15T16:05:10Z".to_string()]
    );
}

#[tokio::test]
async fn cancel_at_deadline_keeps_required_bind_pending() {
    let set = registration_set(vec![
        registration("reg:chanvoy-1", "chanvoy_wait", "chan:a"),
        registration("reg:sms-1", "sms_inbound", "sms:inbox-1"),
    ]);
    let mut request = live_wait_request();
    request.run_deadline = ts("2026-08-15T16:05:00Z");
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = ScriptedObserver::new(Script::default(), clock.clone());
    observer.hang_bind("reg:sms-1");
    let cancel = Cancel::new();
    let policy = CoalescePolicy::new(Duration::from_secs(10));
    let (end, _) = tokio::join!(
        run_coalesce(
            &observer,
            &clock,
            &cancel,
            &set,
            &request,
            &policy,
            |_| async { Ok(()) },
        ),
        async {
            while clock.current_time() < request.run_deadline {
                tokio::task::yield_now().await;
            }
            cancel.trigger();
        }
    );
    let end = end.expect("coalesce");
    assert_eq!(
        end,
        FollowEnd::TerminalArm {
            registration_id: waitprims_core::IdToken::new("reg:sms-1"),
            kind: TerminalArmKind::Failed,
            reason_code: waitprims_core::IdToken::new("required_bind_pending"),
        }
    );
    assert_eq!(observer.live_bind_count(), 0);
}

// ---------------------------------------------------------------------------
// Acceptance proof matrix
// ---------------------------------------------------------------------------

/// Pump a manual clock forward and let the runner observe the advance.
async fn pump(clock: &FakeClock, at: &str) {
    clock.advance_to(&ts(at));
    for _ in 0..96 {
        tokio::task::yield_now().await;
    }
}

#[tokio::test]
async fn scheduled_quiet_sink_err_drops_and_releases_no_replay() {
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
    let clock = FakeClock::manual(request.created_at.clone());
    let observer = ScriptedObserver::new(script, clock.clone());
    let policy = CoalescePolicy::new(Duration::from_secs(10));
    let cancel = Cancel::new();
    let (result, _) = tokio::join!(
        run_coalesce(
            &observer,
            &clock,
            &cancel,
            &set,
            &request,
            &policy,
            |_| async { Err(ValidationError::new("/sink", "refused").into()) },
        ),
        async {
            pump(&clock, "2026-08-15T16:05:00Z").await;
            // Scheduled quiet emit fires on the timer and the sink Err drops it.
            pump(&clock, "2026-08-15T16:05:10Z").await;
            pump(&clock, "2026-08-15T16:20:00Z").await;
        }
    );
    let err = result.expect_err("scheduled quiet sink err");
    assert!(err.to_string().contains("refused"), "{err}");
    assert_eq!(observer.live_bind_count(), 0);
    assert!(
        observer
            .queued_event_ids()
            .get("reg:sms-1")
            .map(Vec::is_empty)
            .unwrap_or(true),
        "scheduled drained burst must not replay: {:?}",
        observer.queued_event_ids()
    );
}

#[tokio::test]
async fn final_flush_sink_err_drops_and_releases_no_replay() {
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
    let clock = FakeClock::manual(request.created_at.clone());
    let observer = ScriptedObserver::new(script, clock.clone());
    let policy = CoalescePolicy::new(Duration::from_secs(3600));
    let cancel = Cancel::new();
    let (result, _) = tokio::join!(
        run_coalesce(
            &observer,
            &clock,
            &cancel,
            &set,
            &request,
            &policy,
            |_| async { Err(ValidationError::new("/sink", "refused").into()) },
        ),
        async {
            pump(&clock, "2026-08-15T16:05:00Z").await;
            // Deadline final_flush drains the buffered quiet event into the sink.
            pump(&clock, "2026-08-15T16:20:00Z").await;
        }
    );
    let err = result.expect_err("final flush sink err");
    assert!(err.to_string().contains("refused"), "{err}");
    assert_eq!(observer.live_bind_count(), 0);
    assert!(
        observer
            .queued_event_ids()
            .get("reg:sms-1")
            .map(Vec::is_empty)
            .unwrap_or(true),
        "final drained burst must not replay: {:?}",
        observer.queued_event_ids()
    );
}

#[tokio::test]
async fn aggregate_event_overflow_restores_custody() {
    let mut set = registration_set(vec![
        registration("reg:a", "sms_inbound", "a"),
        registration("reg:b", "sms_inbound", "b"),
    ]);
    set.aggregate_limits.max_events = 1;
    let request = live_wait_request();
    let at = "2026-08-15T16:05:00Z";
    let script = Script {
        buffer_limit: 8,
        events: vec![
            wait_event("reg:a", "sms_inbound", "evt:a", at),
            wait_event("reg:b", "sms_inbound", "evt:b", at),
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
            registration_id: waitprims_core::IdToken::new("reg:b"),
            kind: TerminalArmKind::Overflow,
            reason_code: waitprims_core::IdToken::new("buffer_overflow"),
        }
    );
    assert_eq!(observer.live_bind_count(), 0);
    assert_eq!(
        observer.queued_event_ids().get("reg:a").map(Vec::as_slice),
        Some(["evt:a".to_string()].as_slice())
    );
    assert_eq!(
        observer.queued_event_ids().get("reg:b").map(Vec::as_slice),
        Some(["evt:b".to_string()].as_slice())
    );
}

#[tokio::test]
async fn aggregate_byte_overflow_restores_custody() {
    let mut set = registration_set(vec![
        registration("reg:a", "sms_inbound", "a"),
        registration("reg:b", "sms_inbound", "b"),
    ]);
    // One event ~= 77 surface bytes; two exceed 100.
    set.aggregate_limits.max_bytes = 100;
    let request = live_wait_request();
    let at = "2026-08-15T16:05:00Z";
    let script = Script {
        buffer_limit: 8,
        events: vec![
            wait_event("reg:a", "sms_inbound", "evt:a", at),
            wait_event("reg:b", "sms_inbound", "evt:b", at),
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
            registration_id: waitprims_core::IdToken::new("reg:b"),
            kind: TerminalArmKind::Overflow,
            reason_code: waitprims_core::IdToken::new("buffer_overflow"),
        }
    );
    assert_eq!(observer.live_bind_count(), 0);
    assert_eq!(
        observer.queued_event_ids().get("reg:a").map(Vec::as_slice),
        Some(["evt:a".to_string()].as_slice())
    );
    assert_eq!(
        observer.queued_event_ids().get("reg:b").map(Vec::as_slice),
        Some(["evt:b".to_string()].as_slice())
    );
}

#[tokio::test]
async fn per_registration_event_overflow_restores_custody() {
    let mut set = registration_set(vec![registration("reg:a", "sms_inbound", "a")]);
    if let Some(reg) = set.registrations.first_mut() {
        reg.bounds.max_events = 1;
    }
    let request = live_wait_request();
    let at = "2026-08-15T16:05:00Z";
    let script = Script {
        buffer_limit: 8,
        events: vec![
            wait_event("reg:a", "sms_inbound", "evt:a1", at),
            wait_event("reg:a", "sms_inbound", "evt:a2", at),
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
            registration_id: waitprims_core::IdToken::new("reg:a"),
            kind: TerminalArmKind::Overflow,
            reason_code: waitprims_core::IdToken::new("buffer_overflow"),
        }
    );
    assert_eq!(observer.live_bind_count(), 0);
    assert_eq!(
        observer.queued_event_ids().get("reg:a").map(Vec::as_slice),
        Some(["evt:a1".to_string(), "evt:a2".to_string()].as_slice())
    );
}

#[tokio::test]
async fn per_registration_byte_overflow_restores_custody() {
    let mut set = registration_set(vec![registration("reg:a", "sms_inbound", "a")]);
    if let Some(reg) = set.registrations.first_mut() {
        reg.bounds.max_bytes = 100;
    }
    let request = live_wait_request();
    let at = "2026-08-15T16:05:00Z";
    let script = Script {
        buffer_limit: 8,
        events: vec![
            wait_event("reg:a", "sms_inbound", "evt:a1", at),
            wait_event("reg:a", "sms_inbound", "evt:a2", at),
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
            registration_id: waitprims_core::IdToken::new("reg:a"),
            kind: TerminalArmKind::Overflow,
            reason_code: waitprims_core::IdToken::new("buffer_overflow"),
        }
    );
    assert_eq!(observer.live_bind_count(), 0);
    assert_eq!(
        observer.queued_event_ids().get("reg:a").map(Vec::as_slice),
        Some(["evt:a1".to_string(), "evt:a2".to_string()].as_slice())
    );
}

#[tokio::test]
async fn mixed_urgent_quiet_overflow_restores_custody() {
    let mut set = registration_set(vec![
        with_priority(
            registration("reg:quiet", "sms_inbound", "q"),
            waitprims_core::PRIORITY_NORMAL,
        ),
        with_priority(
            registration("reg:urgent", "sms_inbound", "u"),
            PRIORITY_URGENT,
        ),
    ]);
    if let Some(quiet) = set
        .registrations
        .iter_mut()
        .find(|reg| reg.registration_id.as_str() == "reg:quiet")
    {
        quiet.bounds.max_events = 2;
    }
    let request = live_wait_request();
    let script = Script {
        buffer_limit: 8,
        events: vec![
            wait_event("reg:quiet", "sms_inbound", "evt:q1", "2026-08-15T16:05:00Z"),
            wait_event("reg:quiet", "sms_inbound", "evt:q2", "2026-08-15T16:06:00Z"),
            wait_event("reg:quiet", "sms_inbound", "evt:q3", "2026-08-15T16:10:00Z"),
            // Urgent lands in the same turn as the quiet overflow trigger, so it
            // is unsunk and must be restored, not emitted.
            wait_event(
                "reg:urgent",
                "sms_inbound",
                "evt:u1",
                "2026-08-15T16:10:00Z",
            ),
        ],
    };
    let clock = FakeClock::manual(request.created_at.clone());
    let observer = ScriptedObserver::new(script, clock.clone());
    let policy = CoalescePolicy::new(Duration::from_secs(3600));
    let cancel = Cancel::new();
    let (end, _) = tokio::join!(
        run_coalesce(
            &observer,
            &clock,
            &cancel,
            &set,
            &request,
            &policy,
            |_| async { Ok(()) }
        ),
        async {
            pump(&clock, "2026-08-15T16:05:00Z").await;
            pump(&clock, "2026-08-15T16:06:00Z").await;
            pump(&clock, "2026-08-15T16:10:00Z").await;
        }
    );
    let end = end.expect("coalesce");
    assert_eq!(
        end,
        FollowEnd::TerminalArm {
            registration_id: waitprims_core::IdToken::new("reg:quiet"),
            kind: TerminalArmKind::Overflow,
            reason_code: waitprims_core::IdToken::new("buffer_overflow"),
        }
    );
    assert_eq!(observer.live_bind_count(), 0);
    assert_eq!(
        observer
            .queued_event_ids()
            .get("reg:quiet")
            .map(Vec::as_slice),
        Some(
            [
                "evt:q1".to_string(),
                "evt:q2".to_string(),
                "evt:q3".to_string()
            ]
            .as_slice()
        ),
        "quiet custody: {:?}",
        observer.queued_event_ids()
    );
    assert_eq!(
        observer
            .queued_event_ids()
            .get("reg:urgent")
            .map(Vec::as_slice),
        Some(["evt:u1".to_string()].as_slice()),
        "urgent unsunk must be restored: {:?}",
        observer.queued_event_ids()
    );
}

#[tokio::test]
async fn final_flush_overflow_restores_custody() {
    let mut set = registration_set(vec![registration("reg:a", "sms_inbound", "a")]);
    if let Some(reg) = set.registrations.first_mut() {
        reg.bounds.max_events = 2;
    }
    let mut request = live_wait_request();
    request.run_deadline = ts("2026-08-15T16:10:00Z");
    let script = Script {
        buffer_limit: 8,
        events: vec![
            wait_event("reg:a", "sms_inbound", "evt:a1", "2026-08-15T16:05:00Z"),
            wait_event("reg:a", "sms_inbound", "evt:a2", "2026-08-15T16:06:00Z"),
            wait_event("reg:a", "sms_inbound", "evt:a3", "2026-08-15T16:10:00Z"),
        ],
    };
    let clock = FakeClock::manual(request.created_at.clone());
    let observer = ScriptedObserver::new(script, clock.clone());
    let policy = CoalescePolicy::new(Duration::from_secs(3600));
    let cancel = Cancel::new();
    let (end, _) = tokio::join!(
        run_coalesce(
            &observer,
            &clock,
            &cancel,
            &set,
            &request,
            &policy,
            |_| async { Ok(()) },
        ),
        async {
            pump(&clock, "2026-08-15T16:05:00Z").await;
            pump(&clock, "2026-08-15T16:06:00Z").await;
            pump(&clock, "2026-08-15T16:10:00Z").await;
        }
    );
    let end = end.expect("coalesce");
    assert_eq!(
        end,
        FollowEnd::TerminalArm {
            registration_id: waitprims_core::IdToken::new("reg:a"),
            kind: TerminalArmKind::Overflow,
            reason_code: waitprims_core::IdToken::new("buffer_overflow"),
        }
    );
    assert_eq!(observer.live_bind_count(), 0);
    assert_eq!(
        observer.queued_event_ids().get("reg:a").map(Vec::as_slice),
        Some(
            [
                "evt:a1".to_string(),
                "evt:a2".to_string(),
                "evt:a3".to_string()
            ]
            .as_slice()
        ),
        "final-flush overflow custody: {:?}",
        observer.queued_event_ids()
    );
}

#[tokio::test]
async fn repeated_urgent_traffic_does_not_starve_quiet() {
    let set = registration_set(vec![
        registration("reg:quiet", "sms_inbound", "q"),
        with_priority(
            registration("reg:urgent", "sms_inbound", "u"),
            PRIORITY_URGENT,
        ),
    ]);
    let request = live_wait_request();
    let script = Script {
        buffer_limit: 8,
        events: vec![
            wait_event("reg:quiet", "sms_inbound", "evt:q", "2026-08-15T16:05:00Z"),
            wait_event(
                "reg:urgent",
                "sms_inbound",
                "evt:u1",
                "2026-08-15T16:05:05Z",
            ),
            wait_event(
                "reg:urgent",
                "sms_inbound",
                "evt:u2",
                "2026-08-15T16:05:07Z",
            ),
            wait_event(
                "reg:urgent",
                "sms_inbound",
                "evt:u3",
                "2026-08-15T16:05:08Z",
            ),
        ],
    };
    let clock = FakeClock::manual(request.created_at.clone());
    let observer = ScriptedObserver::new(script, clock.clone());
    let bursts = Arc::new(Mutex::new(Vec::new()));
    let policy = CoalescePolicy::new(Duration::from_secs(10));
    let cancel = Cancel::new();
    let (end, _) = tokio::join!(
        run_coalesce(&observer, &clock, &cancel, &set, &request, &policy, {
            let bursts = bursts.clone();
            move |burst| {
                push_ids(&bursts, &burst);
                async { Ok(()) }
            }
        }),
        async {
            pump(&clock, "2026-08-15T16:05:00Z").await;
            pump(&clock, "2026-08-15T16:05:05Z").await;
            pump(&clock, "2026-08-15T16:05:07Z").await;
            pump(&clock, "2026-08-15T16:05:08Z").await;
            // Quiet window (16:05:10) must still fire despite urgent traffic.
            pump(&clock, "2026-08-15T16:05:10Z").await;
            pump(&clock, "2026-08-15T16:20:00Z").await;
        }
    );
    let end = end.expect("coalesce");
    assert_eq!(end, FollowEnd::Deadline);
    assert_eq!(
        ids(&bursts),
        vec![
            vec!["evt:u1".to_string()],
            vec!["evt:u2".to_string()],
            vec!["evt:u3".to_string()],
            vec!["evt:q".to_string()],
        ],
        "quiet must flush on its timer despite repeated urgent traffic"
    );
}

#[tokio::test]
async fn multi_registration_multi_turn_burst_order() {
    let set = registration_set(vec![
        registration("reg:a", "sms_inbound", "a"),
        registration("reg:b", "sms_inbound", "b"),
    ]);
    let request = live_wait_request();
    let script = Script {
        buffer_limit: 8,
        events: vec![
            wait_event("reg:a", "sms_inbound", "evt:a1", "2026-08-15T16:05:00Z"),
            wait_event("reg:b", "sms_inbound", "evt:b1", "2026-08-15T16:05:00Z"),
            wait_event("reg:a", "sms_inbound", "evt:a2", "2026-08-15T16:06:00Z"),
        ],
    };
    let clock = FakeClock::manual(request.created_at.clone());
    let observer = ScriptedObserver::new(script, clock.clone());
    let bursts = Arc::new(Mutex::new(Vec::new()));
    let policy = CoalescePolicy::new(Duration::from_secs(3600));
    let cancel = Cancel::new();
    let (end, _) = tokio::join!(
        run_coalesce(&observer, &clock, &cancel, &set, &request, &policy, {
            let bursts = bursts.clone();
            move |burst| {
                push_ids(&bursts, &burst);
                async { Ok(()) }
            }
        }),
        async {
            pump(&clock, "2026-08-15T16:05:00Z").await;
            pump(&clock, "2026-08-15T16:06:00Z").await;
            pump(&clock, "2026-08-15T16:20:00Z").await;
        }
    );
    let end = end.expect("coalesce");
    assert_eq!(end, FollowEnd::Deadline);
    // Turn FIFO (16:05 then 16:06), registration-set order within a turn
    // (reg:a before reg:b), per-registration FIFO (a1 before a2).
    assert_eq!(
        ids(&bursts),
        vec![vec![
            "evt:a1".to_string(),
            "evt:b1".to_string(),
            "evt:a2".to_string(),
        ]]
    );
}

/// Observer wrapper that records every `next` invocation in order.
struct NextCounting {
    inner: ScriptedObserver,
    calls: Arc<Mutex<Vec<String>>>,
}

impl Observer for NextCounting {
    type Bind = TrackedBind;

    async fn bind(&self, registration: &Registration) -> Result<Self::Bind> {
        self.inner.bind(registration).await
    }

    async fn next(&self, bind: &Self::Bind) -> Result<Observation> {
        self.calls
            .lock()
            .expect("calls")
            .push(bind.registration_id.as_str().to_string());
        self.inner.next(bind).await
    }

    async fn cancel(&self, bind: &Self::Bind) -> Result<()> {
        self.inner.cancel(bind).await
    }

    fn restore_ready(&self, bind: &Self::Bind, obs: Observation) -> Result<()> {
        self.inner.restore_ready(bind, obs)
    }
}

/// Held binds + backpressure across more than two successful emits:
/// `Observer::next` is not called again until the prior `on_burst` returns `Ok`.
#[tokio::test]
async fn held_binds_backpressure_across_multiple_emits() {
    let set = registration_set(vec![with_priority(
        registration("reg:sms-1", "sms_inbound", "sms:inbox-1"),
        PRIORITY_URGENT,
    )]);
    let request = live_wait_request();
    let script = Script {
        buffer_limit: 8,
        events: vec![
            wait_event("reg:sms-1", "sms_inbound", "evt:1", "2026-08-15T16:05:00Z"),
            wait_event("reg:sms-1", "sms_inbound", "evt:2", "2026-08-15T16:06:00Z"),
            wait_event("reg:sms-1", "sms_inbound", "evt:3", "2026-08-15T16:07:00Z"),
        ],
    };
    let clock = FakeClock::manual(request.created_at.clone());
    let inner = ScriptedObserver::new(script, clock.clone());
    let calls = Arc::new(Mutex::new(Vec::new()));
    let observer = NextCounting {
        inner: inner.clone(),
        calls: calls.clone(),
    };
    let bursts = Arc::new(Mutex::new(Vec::new()));
    let (gate_tx, gate_rx) = tokio::sync::oneshot::channel::<()>();
    let gate_rx = Mutex::new(Some(gate_rx));
    let policy = CoalescePolicy::new(Duration::from_secs(10));
    let cancel = Cancel::new();
    let (end, _) = tokio::join!(
        run_coalesce(&observer, &clock, &cancel, &set, &request, &policy, {
            let bursts = bursts.clone();
            move |burst| {
                let first = bursts.lock().expect("bursts").is_empty();
                push_ids(&bursts, &burst);
                let rx = if first {
                    gate_rx.lock().expect("gate").take()
                } else {
                    None
                };
                async move {
                    if let Some(rx) = rx {
                        let _ = rx.await;
                    }
                    Ok(())
                }
            }
        }),
        async {
            pump(&clock, "2026-08-15T16:05:00Z").await;
            while bursts.lock().expect("bursts").is_empty() {
                tokio::task::yield_now().await;
            }
            assert_eq!(bursts.lock().expect("bursts").len(), 1);
            // Advance the clock past the next events: backpressure must hold
            // `next` until the first `on_burst` returns Ok.
            pump(&clock, "2026-08-15T16:06:00Z").await;
            pump(&clock, "2026-08-15T16:07:00Z").await;
            tokio::task::yield_now().await;
            assert_eq!(
                calls.lock().expect("calls").len(),
                1,
                "next must stay parked while the first on_burst is pending"
            );
            assert_eq!(inner.live_bind_count(), 1);
            let _ = gate_tx.send(());
            while bursts.lock().expect("bursts").len() < 3 {
                tokio::task::yield_now().await;
            }
            pump(&clock, "2026-08-15T16:20:00Z").await;
        }
    );
    let end = end.expect("coalesce");
    assert_eq!(end, FollowEnd::Deadline);
    assert_eq!(
        ids(&bursts),
        vec![
            vec!["evt:1".to_string()],
            vec!["evt:2".to_string()],
            vec!["evt:3".to_string()],
        ]
    );
    // Three emissions drove three `next` calls; the runner polls `next` once
    // more to discover Idle and reach the deadline, so the total is four.
    assert_eq!(calls.lock().expect("calls").len(), 4);
    assert_eq!(inner.live_bind_count(), 0);
}
