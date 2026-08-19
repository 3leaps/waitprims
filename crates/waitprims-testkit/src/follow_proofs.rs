//! Proofs: held-follow lifecycle and lease boundary.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use waitprims_async::{
    run_follow, BindHandle, Cancel, FollowEnd, Observation, Observer, TerminalArmKind,
};
use waitprims_core::{
    AuthnMode, Error, IdToken, NormativeReason, OpaqueRef, Registration, Result, ValidationError,
    WaitEvent,
};

use crate::{
    live_wait_request, registration, registration_set, resolve_start_at_bind, ts, wait_event,
    BindTracker, EndlessReadyObserver, FakeClock, Script, ScriptedObserver, TrackedBind,
};

fn two_arm_set() -> waitprims_core::RegistrationSet {
    registration_set(vec![
        registration("reg:chanvoy-1", "chanvoy_wait", "chan:seat-a"),
        registration("reg:sms-1", "sms_inbound", "sms:inbox-1"),
    ])
}

fn norm(err: &Error) -> &ValidationError {
    match err {
        Error::Validation(err) => err,
        other => panic!("expected validation error, got {other}"),
    }
}

async fn ok_sink(_burst: waitprims_async::FollowBurst) -> Result<()> {
    Ok(())
}

struct CountBinds {
    inner: ScriptedObserver,
    binds: Arc<AtomicU64>,
}

impl Observer for CountBinds {
    type Bind = TrackedBind;

    async fn bind(&self, registration: &Registration) -> Result<Self::Bind> {
        self.binds.fetch_add(1, Ordering::Relaxed);
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

    fn restore_ready(&self, bind: &Self::Bind, obs: Observation) -> Result<()> {
        self.inner.restore_ready(bind, obs)
    }
}

struct CancelOnNext {
    inner: ScriptedObserver,
    cancel: Cancel,
}

impl Observer for CancelOnNext {
    type Bind = TrackedBind;

    async fn bind(&self, registration: &Registration) -> Result<Self::Bind> {
        self.inner.bind(registration).await
    }

    async fn next(&self, bind: &Self::Bind) -> Result<Observation> {
        let obs = self.inner.next(bind).await?;
        self.cancel.trigger();
        Ok(obs)
    }

    async fn cancel(&self, bind: &Self::Bind) -> Result<()> {
        self.inner.cancel(bind).await
    }

    fn restore_ready(&self, bind: &Self::Bind, obs: Observation) -> Result<()> {
        self.inner.restore_ready(bind, obs)
    }
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
async fn two_emissions_same_binds() {
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
    let inner = ScriptedObserver::new(script, clock.clone());
    let binds = Arc::new(AtomicU64::new(0));
    let observer = CountBinds {
        inner: inner.clone(),
        binds: binds.clone(),
    };
    let cancel = Cancel::new();
    let bursts = Arc::new(Mutex::new(Vec::new()));
    let (end, _) = tokio::join!(
        run_follow(&observer, &clock, &cancel, &set, &request, {
            let bursts = bursts.clone();
            let cancel = cancel.clone();
            move |burst| {
                let ids: Vec<String> = burst
                    .events
                    .iter()
                    .map(|event| event.event_id.as_str().to_string())
                    .collect();
                bursts.lock().expect("bursts").push(ids);
                if bursts.lock().expect("bursts").len() >= 2 {
                    cancel.trigger();
                }
                async { Ok(()) }
            }
        }),
        async {
            while bursts.lock().expect("bursts").len() < 2 {
                tokio::task::yield_now().await;
            }
        }
    );
    assert_eq!(end.expect("follow"), FollowEnd::Cancel);
    assert_eq!(
        *bursts.lock().expect("bursts"),
        vec![vec!["evt:1".to_string()], vec!["evt:2".to_string()]]
    );
    assert_eq!(binds.load(Ordering::Relaxed), 1);
    assert_eq!(inner.live_bind_count(), 0);
}

#[tokio::test]
async fn same_instant_burst_is_registration_order() {
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
    let bursts = Arc::new(Mutex::new(Vec::new()));
    let end = run_follow(&observer, &clock, &cancel, &set, &request, {
        let bursts = bursts.clone();
        let cancel = cancel.clone();
        move |burst| {
            let ids: Vec<String> = burst
                .events
                .iter()
                .map(|event| event.event_id.as_str().to_string())
                .collect();
            bursts.lock().expect("bursts").push(ids);
            cancel.trigger();
            async { Ok(()) }
        }
    })
    .await
    .expect("follow");
    assert_eq!(end, FollowEnd::Cancel);
    assert_eq!(
        *bursts.lock().expect("bursts"),
        vec![vec!["evt:chanvoy-1".to_string(), "evt:sms-1".to_string()]]
    );
    for event in observer.queued_event_ids().values() {
        assert!(event.is_empty());
    }
    assert_eq!(observer.live_bind_count(), 0);
}

#[tokio::test]
async fn event_and_terminal_same_turn_emits_then_ends() {
    let set = two_arm_set();
    let request = live_wait_request();
    let at = "2026-08-15T16:05:00Z";
    let script = Script {
        buffer_limit: 1,
        events: vec![
            wait_event("reg:chanvoy-1", "chanvoy_wait", "evt:chanvoy-1", at),
            wait_event("reg:sms-1", "sms_inbound", "evt:sms-a", at),
            wait_event("reg:sms-1", "sms_inbound", "evt:sms-b", at),
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
    assert_eq!(
        *bursts.lock().expect("bursts"),
        vec![vec!["evt:chanvoy-1".to_string()]]
    );
    assert_eq!(
        end,
        FollowEnd::TerminalArm {
            registration_id: IdToken::new("reg:sms-1"),
            kind: TerminalArmKind::Overflow,
            reason_code: IdToken::new("buffer_overflow"),
        }
    );
    assert_eq!(observer.live_bind_count(), 0);
}

async fn prove_terminal(kind: TerminalArmKind, arm: impl Fn(&ScriptedObserver)) {
    let set = registration_set(vec![registration(
        "reg:sms-1",
        "sms_inbound",
        "sms:inbox-1",
    )]);
    let request = live_wait_request();
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = ScriptedObserver::new(Script::default(), clock.clone());
    arm(&observer);
    let end = run_follow(&observer, &clock, &Cancel::new(), &set, &request, ok_sink)
        .await
        .expect("terminal");
    match end {
        FollowEnd::TerminalArm {
            registration_id,
            kind: got,
            reason_code,
        } => {
            assert_eq!(registration_id.as_str(), "reg:sms-1");
            assert_eq!(got, kind);
            assert_eq!(reason_code.as_str(), "arm_down");
        }
        other => panic!("unexpected end {other:?}"),
    }
    assert_eq!(observer.live_bind_count(), 0);
}

#[tokio::test]
async fn each_terminal_arm_kind_ends_follow() {
    prove_terminal(TerminalArmKind::Failed, |obs| {
        obs.fail_arm("reg:sms-1", "arm_down");
    })
    .await;
    prove_terminal(TerminalArmKind::Outage, |obs| {
        obs.outage("reg:sms-1", "arm_down");
    })
    .await;
    prove_terminal(TerminalArmKind::CursorUncertain, |obs| {
        obs.cursor_uncertain("reg:sms-1", "arm_down");
    })
    .await;
    prove_terminal(TerminalArmKind::Degraded, |obs| {
        obs.degrade("reg:sms-1", "arm_down");
    })
    .await;
}

#[tokio::test]
async fn deadline_emits_ready_burst_then_deadline() {
    let set = registration_set(vec![registration(
        "reg:sms-1",
        "sms_inbound",
        "sms:inbox-1",
    )]);
    let mut request = live_wait_request();
    request.run_deadline = ts("2026-08-15T16:05:00Z");
    request.logical_deadline = ts("2026-08-15T17:00:00Z");
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
        *bursts.lock().expect("bursts"),
        vec![vec!["evt:sms-1".to_string()]]
    );
    assert!(observer
        .queued_event_ids()
        .get("reg:sms-1")
        .map(Vec::is_empty)
        .unwrap_or(true));
    assert_eq!(observer.live_bind_count(), 0);
}

#[tokio::test]
async fn sink_backpressure_holds_next_until_ok() {
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
    let cancel = Cancel::new();
    let bursts = Arc::new(Mutex::new(Vec::new()));
    let (gate_tx, gate_rx) = tokio::sync::oneshot::channel::<()>();
    let gate_rx = Mutex::new(Some(gate_rx));
    let (end, _) = tokio::join!(
        run_follow(&observer, &clock, &cancel, &set, &request, {
            let bursts = bursts.clone();
            move |burst| {
                let first = bursts.lock().expect("bursts").is_empty();
                bursts.lock().expect("bursts").push(
                    burst
                        .events
                        .iter()
                        .map(|event| event.event_id.as_str().to_string())
                        .collect::<Vec<_>>(),
                );
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
            while bursts.lock().expect("bursts").is_empty() {
                tokio::task::yield_now().await;
            }
            assert_eq!(bursts.lock().expect("bursts").len(), 1);
            assert_eq!(observer.live_bind_count(), 1);
            let _ = gate_tx.send(());
            while bursts.lock().expect("bursts").len() < 2 {
                tokio::task::yield_now().await;
            }
            cancel.trigger();
        }
    );
    assert_eq!(end.expect("follow"), FollowEnd::Cancel);
    assert_eq!(
        *bursts.lock().expect("bursts"),
        vec![vec!["evt:1".to_string()], vec!["evt:2".to_string()]]
    );
    assert_eq!(observer.live_bind_count(), 0);
}

#[tokio::test]
async fn sink_error_releases_binds_without_restore() {
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
            "evt:sms-1",
            "2026-08-15T16:05:00Z",
        )],
    };
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = ScriptedObserver::new(script, clock.clone());
    let err = run_follow(
        &observer,
        &clock,
        &Cancel::new(),
        &set,
        &request,
        |_| async { Err(ValidationError::new("/sink", "refused").into()) },
    )
    .await
    .expect_err("sink err");
    assert!(err.to_string().contains("refused"), "{err}");
    assert!(
        observer
            .queued_event_ids()
            .get("reg:sms-1")
            .map(Vec::is_empty)
            .unwrap_or(true),
        "sink Err must not restore the burst: {:?}",
        observer.queued_event_ids()
    );
    assert_eq!(observer.live_bind_count(), 0);
}

#[tokio::test]
async fn pre_sink_cancel_restores_unemitted() {
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
            "evt:sms-1",
            "2026-08-15T16:05:00Z",
        )],
    };
    let clock = FakeClock::auto(request.created_at.clone());
    let inner = ScriptedObserver::new(script, clock.clone());
    let cancel = Cancel::new();
    let observer = CancelOnNext {
        inner: inner.clone(),
        cancel: cancel.clone(),
    };
    let bursts = Arc::new(Mutex::new(Vec::<String>::new()));
    let end = run_follow(&observer, &clock, &cancel, &set, &request, {
        let bursts = bursts.clone();
        move |burst| {
            bursts.lock().expect("bursts").extend(
                burst
                    .events
                    .iter()
                    .map(|event| event.event_id.as_str().to_string()),
            );
            async { Ok(()) }
        }
    })
    .await
    .expect("cancel");
    assert_eq!(end, FollowEnd::Cancel);
    assert!(
        bursts.lock().expect("bursts").is_empty(),
        "pre-sink cancel must not emit: {:?}",
        bursts.lock().expect("bursts")
    );
    assert_eq!(
        inner.queued_event_ids().get("reg:sms-1").map(Vec::as_slice),
        Some(["evt:sms-1".to_string()].as_slice()),
        "unemitted event must be restored: {:?}",
        inner.queued_event_ids()
    );
    assert_eq!(inner.live_bind_count(), 0);
}

#[tokio::test]
async fn pending_required_bind_at_deadline_is_terminal_failed() {
    let set = registration_set(vec![registration(
        "reg:sms-1",
        "sms_inbound",
        "sms:inbox-1",
    )]);
    let mut request = live_wait_request();
    request.run_deadline = ts("2026-08-15T16:02:00Z");
    request.logical_deadline = ts("2026-08-15T17:00:00Z");
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = ScriptedObserver::new(Script::default(), clock.clone());
    observer.hang_bind("reg:sms-1");
    let end = run_follow(&observer, &clock, &Cancel::new(), &set, &request, ok_sink)
        .await
        .expect("pending bind");
    assert_eq!(
        end,
        FollowEnd::TerminalArm {
            registration_id: IdToken::new("reg:sms-1"),
            kind: TerminalArmKind::Failed,
            reason_code: IdToken::new("required_bind_pending"),
        }
    );
    assert_eq!(observer.live_bind_count(), 0);
}

#[tokio::test]
async fn deadline_emits_ready_burst_before_pending_required_bind() {
    let set = two_arm_set();
    let mut request = live_wait_request();
    request.run_deadline = ts("2026-08-15T16:02:00Z");
    request.logical_deadline = ts("2026-08-15T17:00:00Z");
    let script = Script {
        buffer_limit: 8,
        events: vec![wait_event(
            "reg:sms-1",
            "sms_inbound",
            "evt:sms-1",
            "2026-08-15T16:02:00Z",
        )],
    };
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = ScriptedObserver::new(script, clock.clone());
    observer.hang_bind("reg:chanvoy-1");
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
    assert_eq!(
        *bursts.lock().expect("bursts"),
        vec![vec!["evt:sms-1".to_string()]]
    );
    assert_eq!(
        end,
        FollowEnd::TerminalArm {
            registration_id: IdToken::new("reg:chanvoy-1"),
            kind: TerminalArmKind::Failed,
            reason_code: IdToken::new("required_bind_pending"),
        }
    );
    assert!(
        observer
            .queued_event_ids()
            .get("reg:sms-1")
            .map(Vec::is_empty)
            .unwrap_or(true),
        "emitted sibling event must not be restored: {:?}",
        observer.queued_event_ids()
    );
    assert_eq!(observer.live_bind_count(), 0);
}

#[tokio::test]
async fn drop_of_follow_future_releases_binds() {
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
            "evt:sms-1",
            "2026-08-15T16:05:00Z",
        )],
    };
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = ScriptedObserver::new(script, clock.clone());
    let started = Arc::new(Mutex::new(false));
    let cancel = Cancel::new();
    tokio::select! {
        biased;
        result = run_follow(&observer, &clock, &cancel, &set, &request, {
            let started = started.clone();
            move |_| {
                *started.lock().expect("started") = true;
                async {
                    std::future::pending::<()>().await;
                    Ok(())
                }
            }
        }) => panic!("dropped follow must not complete: {result:?}"),
        _ = async {
            while !*started.lock().expect("started") {
                tokio::task::yield_now().await;
            }
        } => {}
    }
    assert_eq!(observer.live_bind_count(), 0);
    assert!(
        observer
            .queued_event_ids()
            .get("reg:sms-1")
            .map(Vec::is_empty)
            .unwrap_or(true),
        "in-flight sink abort must not restore: {:?}",
        observer.queued_event_ids()
    );
}

#[tokio::test]
async fn lease_rejects_at_or_after_expiry_before_sink() {
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
    let bursts = Arc::new(Mutex::new(Vec::<String>::new()));
    let err = run_follow(&observer, &clock, &Cancel::new(), &set, &request, {
        let bursts = bursts.clone();
        move |burst| {
            bursts.lock().expect("bursts").extend(
                burst
                    .events
                    .iter()
                    .map(|event| event.event_id.as_str().to_string()),
            );
            async { Ok(()) }
        }
    })
    .await
    .expect_err("lease");
    let err = norm(&err);
    assert_eq!(err.reason, Some(NormativeReason::LeaseReauth));
    assert_eq!(err.path, "/registrations/lease_expires_at");
    assert_eq!(err.constraint, "lease_expired");
    assert!(bursts.lock().expect("bursts").is_empty());
    assert_eq!(
        observer
            .queued_event_ids()
            .get("reg:sms-1")
            .map(Vec::as_slice),
        Some(["evt:lease-1".to_string()].as_slice()),
        "event at lease boundary must be restored: {:?}",
        observer.queued_event_ids()
    );
    assert_eq!(observer.live_bind_count(), 0);
}

#[tokio::test]
async fn quiet_lease_boundary_does_not_wait_on_provider() {
    let mut leased = registration("reg:sms-1", "sms_inbound", "sms:inbox-1");
    leased.lease_expires_at = ts("2026-08-15T16:02:00Z");
    let set = registration_set(vec![leased]);
    let request = live_wait_request();
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = ScriptedObserver::new(Script::default(), clock.clone());
    let err = run_follow(&observer, &clock, &Cancel::new(), &set, &request, ok_sink)
        .await
        .expect_err("lease");
    let err = norm(&err);
    assert_eq!(err.reason, Some(NormativeReason::LeaseReauth));
    assert!(clock.current_time() >= ts("2026-08-15T16:02:00Z"));
    assert!(clock.current_time() < request.run_deadline);
    assert_eq!(observer.live_bind_count(), 0);
}

#[tokio::test]
async fn authn_required_is_normative_and_distinct() {
    let mut set = registration_set(vec![registration(
        "reg:sms-1",
        "sms_inbound",
        "sms:inbox-1",
    )]);
    set.authn_mode = AuthnMode::Required;
    let request = live_wait_request();
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = ScriptedObserver::new(Script::default(), clock.clone());
    let err = run_follow(&observer, &clock, &Cancel::new(), &set, &request, ok_sink)
        .await
        .expect_err("authn");
    let err = norm(&err);
    assert_eq!(err.reason, Some(NormativeReason::AuthnRequired));
    assert_eq!(err.path, "/verification_receipt_ref");
    assert_eq!(err.constraint, "required");
    assert_eq!(observer.live_bind_count(), 0);

    let mut allowed = request.clone();
    allowed.verification_receipt_ref = Some(OpaqueRef::new("vr:seat-a"));
    let cancel = Cancel::new();
    let end = run_follow(&observer, &clock, &cancel, &set, &allowed, ok_sink)
        .await
        .expect("deadline after receipt");
    assert_eq!(end, FollowEnd::Deadline);
}

#[tokio::test]
async fn observer_error_restores_sibling_event() {
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
    let err = run_follow(&observer, &clock, &Cancel::new(), &set, &request, ok_sink)
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
async fn restore_error_fail_closes_and_releases() {
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
    let cancel = Cancel::new();
    let observer = RestoreFailOnCancel {
        inner: inner.clone(),
        cancel: cancel.clone(),
    };
    let err = run_follow(&observer, &clock, &cancel, &set, &request, ok_sink)
        .await
        .expect_err("restore");
    assert!(err.to_string().contains("restore_failed"), "{err}");
    assert_eq!(inner.live_bind_count(), 0);
}

/// Returns Idle until `event.observed_at`, then one Event via next or poll_ready.
struct IdleThenReady {
    tracker: BindTracker,
    clock: FakeClock,
    event: Mutex<Option<WaitEvent>>,
}

impl IdleThenReady {
    fn new(clock: FakeClock, event: WaitEvent) -> Self {
        Self {
            tracker: BindTracker::new(),
            clock,
            event: Mutex::new(Some(event)),
        }
    }

    fn live_bind_count(&self) -> usize {
        self.tracker.live_count()
    }

    fn queued_event_id(&self) -> Option<String> {
        self.event
            .lock()
            .expect("event")
            .as_ref()
            .map(|event| event.event_id.as_str().to_string())
    }

    fn take_due(&self) -> Option<Observation> {
        let now = self.clock.current_time();
        let mut slot = self.event.lock().expect("event");
        let event = slot.as_ref()?;
        if event.observed_at <= now {
            return slot.take().map(|event| Observation::Event(Box::new(event)));
        }
        None
    }
}

impl Observer for IdleThenReady {
    type Bind = TrackedBind;

    async fn bind(&self, registration: &Registration) -> Result<Self::Bind> {
        self.tracker.acquire(registration.registration_id.as_str());
        Ok(TrackedBind::new(
            registration.registration_id.clone(),
            resolve_start_at_bind(registration, None),
            self.tracker.clone(),
        ))
    }

    async fn next(&self, _bind: &Self::Bind) -> Result<Observation> {
        Ok(self.take_due().unwrap_or(Observation::Idle))
    }

    async fn cancel(&self, bind: &Self::Bind) -> Result<()> {
        self.tracker.cancel(bind.registration_id.as_str());
        Ok(())
    }

    fn poll_ready(&self, _bind: &Self::Bind) -> Option<Observation> {
        self.take_due()
    }

    fn restore_ready(&self, _bind: &Self::Bind, obs: Observation) -> Result<()> {
        if let Observation::Event(event) = obs {
            *self.event.lock().expect("event") = Some(*event);
        }
        Ok(())
    }
}

struct RestoreFailOnCancel {
    inner: ScriptedObserver,
    cancel: Cancel,
}

impl Observer for RestoreFailOnCancel {
    type Bind = TrackedBind;

    async fn bind(&self, registration: &Registration) -> Result<Self::Bind> {
        self.inner.bind(registration).await
    }

    async fn next(&self, bind: &Self::Bind) -> Result<Observation> {
        let obs = self.inner.next(bind).await?;
        self.cancel.trigger();
        Ok(obs)
    }

    async fn cancel(&self, bind: &Self::Bind) -> Result<()> {
        self.inner.cancel(bind).await
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
async fn one_observation_per_slot_per_turn() {
    let set = two_arm_set();
    let request = live_wait_request();
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = EndlessReadyObserver::new();
    let cancel = Cancel::new();
    let sizes = Arc::new(Mutex::new(Vec::new()));
    let end = run_follow(&observer, &clock, &cancel, &set, &request, {
        let sizes = sizes.clone();
        let cancel = cancel.clone();
        move |burst| {
            sizes.lock().expect("sizes").push(burst.events.len());
            for event in &burst.events {
                assert!(event
                    .proposed_next_anchor
                    .value
                    .as_str()
                    .starts_with("anc:"));
            }
            cancel.trigger();
            async { Ok(()) }
        }
    })
    .await
    .expect("follow");
    assert_eq!(end, FollowEnd::Cancel);
    let sizes = sizes.lock().expect("sizes");
    assert_eq!(sizes.as_slice(), &[2]);
    assert_eq!(observer.live_bind_count(), 0);
}

#[tokio::test]
async fn idle_then_event_at_deadline_emits_burst() {
    let set = registration_set(vec![registration(
        "reg:sms-1",
        "sms_inbound",
        "sms:inbox-1",
    )]);
    let mut request = live_wait_request();
    request.run_deadline = ts("2026-08-15T16:05:00Z");
    request.logical_deadline = ts("2026-08-15T17:00:00Z");
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = IdleThenReady::new(
        clock.clone(),
        wait_event(
            "reg:sms-1",
            "sms_inbound",
            "evt:sms-1",
            "2026-08-15T16:05:00Z",
        ),
    );
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
        *bursts.lock().expect("bursts"),
        vec![vec!["evt:sms-1".to_string()]]
    );
    assert!(observer.queued_event_id().is_none());
    assert_eq!(observer.live_bind_count(), 0);
}

#[tokio::test]
async fn idle_then_event_at_lease_restores_without_sink() {
    let mut leased = registration("reg:sms-1", "sms_inbound", "sms:inbox-1");
    leased.lease_expires_at = ts("2026-08-15T16:02:00Z");
    let set = registration_set(vec![leased]);
    let request = live_wait_request();
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = IdleThenReady::new(
        clock.clone(),
        wait_event(
            "reg:sms-1",
            "sms_inbound",
            "evt:lease-1",
            "2026-08-15T16:02:00Z",
        ),
    );
    let bursts = Arc::new(Mutex::new(Vec::<String>::new()));
    let err = run_follow(&observer, &clock, &Cancel::new(), &set, &request, {
        let bursts = bursts.clone();
        move |burst| {
            bursts.lock().expect("bursts").extend(
                burst
                    .events
                    .iter()
                    .map(|event| event.event_id.as_str().to_string()),
            );
            async { Ok(()) }
        }
    })
    .await
    .expect_err("lease");
    let err = norm(&err);
    assert_eq!(err.reason, Some(NormativeReason::LeaseReauth));
    assert!(bursts.lock().expect("bursts").is_empty());
    assert_eq!(observer.queued_event_id().as_deref(), Some("evt:lease-1"));
    assert_eq!(observer.live_bind_count(), 0);
}

#[tokio::test]
async fn proposed_next_anchor_survives_emit() {
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
            "evt:sms-1",
            "2026-08-15T16:05:00Z",
        )],
    };
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = ScriptedObserver::new(script, clock.clone());
    let cancel = Cancel::new();
    let kept = Arc::new(Mutex::new(None));
    let end = run_follow(&observer, &clock, &cancel, &set, &request, {
        let kept = kept.clone();
        let cancel = cancel.clone();
        move |burst| {
            *kept.lock().expect("kept") = Some(
                burst.events[0]
                    .proposed_next_anchor
                    .value
                    .as_str()
                    .to_string(),
            );
            cancel.trigger();
            async { Ok(()) }
        }
    })
    .await
    .expect("follow");
    assert_eq!(end, FollowEnd::Cancel);
    assert_eq!(kept.lock().expect("kept").as_deref(), Some("anc:after-1"));
}
