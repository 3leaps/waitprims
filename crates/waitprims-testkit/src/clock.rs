//! Deterministic logical clock.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::oneshot;
use waitprims_async::Clock;
use waitprims_core::Timestamp;

struct Sleeper {
    id: u64,
    deadline: Timestamp,
    tx: oneshot::Sender<()>,
}

struct Inner {
    now: Mutex<Timestamp>,
    sleepers: Mutex<Vec<Sleeper>>,
    next_id: AtomicU64,
    auto: AtomicBool,
}

/// Logical clock for scripted waits. Does not use wall time.
#[derive(Clone)]
pub struct FakeClock {
    inner: Arc<Inner>,
}

impl FakeClock {
    /// Clock that advances to the earliest sleeper once all sleeps have registered.
    pub fn auto(now: Timestamp) -> Self {
        Self::new(now, true)
    }

    /// Clock that advances only when [`Self::advance_to`] is called.
    pub fn manual(now: Timestamp) -> Self {
        Self::new(now, false)
    }

    fn new(now: Timestamp, auto: bool) -> Self {
        Self {
            inner: Arc::new(Inner {
                now: Mutex::new(now),
                sleepers: Mutex::new(Vec::new()),
                next_id: AtomicU64::new(1),
                auto: AtomicBool::new(auto),
            }),
        }
    }

    /// Current logical time.
    pub fn current_time(&self) -> Timestamp {
        self.inner.now.lock().expect("clock").clone()
    }

    /// Number of waits currently parked on this clock.
    pub fn sleeper_count(&self) -> usize {
        self.inner.sleepers.lock().expect("clock").len()
    }

    /// Advance to `deadline` if it is later than now, then wake due sleepers.
    pub fn advance_to(&self, deadline: &Timestamp) {
        {
            let mut now = self.inner.now.lock().expect("clock");
            if *deadline > *now {
                *now = deadline.clone();
            }
        }
        self.wake_ready();
    }

    /// Wait until `deadline` in logical time.
    pub async fn sleep_to(&self, deadline: &Timestamp) {
        if self.current_time() >= *deadline {
            return;
        }
        let (tx, rx) = oneshot::channel();
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        self.inner.sleepers.lock().expect("clock").push(Sleeper {
            id,
            deadline: deadline.clone(),
            tx,
        });
        let guard = SleeperGuard {
            clock: self.clone(),
            id,
        };
        tokio::task::yield_now().await;
        if self.inner.auto.load(Ordering::Relaxed) {
            self.auto_advance_if_earliest(deadline);
        }
        let _ = rx.await;
        drop(guard);
    }

    fn auto_advance_if_earliest(&self, mine: &Timestamp) {
        let min = {
            let sleepers = self.inner.sleepers.lock().expect("clock");
            sleepers
                .iter()
                .map(|sleeper| sleeper.deadline.clone())
                .min()
        };
        if min.as_ref() == Some(mine) {
            self.advance_to(mine);
        }
    }

    fn wake_ready(&self) {
        let now = self.current_time();
        let mut sleepers = self.inner.sleepers.lock().expect("clock");
        let mut idx = 0;
        while idx < sleepers.len() {
            if sleepers[idx].deadline <= now {
                let sleeper = sleepers.remove(idx);
                let _ = sleeper.tx.send(());
            } else {
                idx += 1;
            }
        }
    }

    fn remove(&self, id: u64) {
        self.inner
            .sleepers
            .lock()
            .expect("clock")
            .retain(|sleeper| sleeper.id != id);
    }
}

struct SleeperGuard {
    clock: FakeClock,
    id: u64,
}

impl Drop for SleeperGuard {
    fn drop(&mut self) {
        self.clock.remove(self.id);
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Timestamp {
        self.current_time()
    }

    fn sleep_until(&self, deadline: &Timestamp) -> impl std::future::Future<Output = ()> + Send {
        let clock = self.clone();
        let deadline = deadline.clone();
        async move {
            clock.sleep_to(&deadline).await;
        }
    }
}
