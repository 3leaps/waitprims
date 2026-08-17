//! First-ready collection across observer arms.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::observer::Observation;
use waitprims_core::{Error, Result};

enum Arm<F, T> {
    Pending(Pin<Box<F>>),
    Ready(Option<T>),
    Failed,
}

/// Collected arm values, plus an error if any arm failed after others were ready.
pub(crate) struct FirstReadyOutput<T> {
    pub ready: Vec<(usize, T)>,
    pub error: Option<Error>,
}

/// Wait until at least one arm yields a terminal value, or every arm is ready
/// with a non-terminal value (all idle). Already-ready values are harvested
/// even when a sibling returns an error so the runner can restore them.
pub(crate) struct FirstReady<F, T> {
    arms: Vec<Arm<F, T>>,
    is_terminal: fn(&T) -> bool,
}

impl<F, T> FirstReady<F, T>
where
    F: Future<Output = Result<T>>,
{
    pub(crate) fn new(futures: impl IntoIterator<Item = F>, is_terminal: fn(&T) -> bool) -> Self {
        Self {
            arms: futures
                .into_iter()
                .map(|fut| Arm::Pending(Box::pin(fut)))
                .collect(),
            is_terminal,
        }
    }

    /// Take values that completed before this future finished (cancel path).
    pub(crate) fn take_ready(&mut self) -> Vec<(usize, T)> {
        let mut out = Vec::new();
        for (idx, arm) in self.arms.iter_mut().enumerate() {
            if let Arm::Ready(slot) = arm {
                if let Some(value) = slot.take() {
                    out.push((idx, value));
                }
            }
        }
        out
    }
}

impl<F, T> Future for FirstReady<F, T>
where
    F: Future<Output = Result<T>>,
    T: Unpin,
{
    type Output = FirstReadyOutput<T>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let mut pending = false;
        let mut terminal = false;
        let mut error = None;

        for arm in &mut this.arms {
            match arm {
                Arm::Pending(fut) => match fut.as_mut().poll(cx) {
                    Poll::Ready(Ok(value)) => {
                        if (this.is_terminal)(&value) {
                            terminal = true;
                        }
                        *arm = Arm::Ready(Some(value));
                    }
                    Poll::Ready(Err(err)) => {
                        if error.is_none() {
                            error = Some(err);
                        }
                        *arm = Arm::Failed;
                    }
                    Poll::Pending => pending = true,
                },
                Arm::Ready(Some(value)) => {
                    if (this.is_terminal)(value) {
                        terminal = true;
                    }
                }
                Arm::Ready(None) | Arm::Failed => {}
            }
        }

        if error.is_some() || terminal || !pending {
            let mut out = Vec::new();
            for (idx, arm) in this.arms.iter_mut().enumerate() {
                match arm {
                    Arm::Ready(slot) => {
                        if let Some(value) = slot.take() {
                            out.push((idx, value));
                        }
                    }
                    Arm::Pending(fut) => match fut.as_mut().poll(cx) {
                        Poll::Ready(Ok(value)) => out.push((idx, value)),
                        Poll::Ready(Err(err)) => {
                            if error.is_none() {
                                error = Some(err);
                            }
                        }
                        Poll::Pending => {}
                    },
                    Arm::Failed => {}
                }
            }
            return Poll::Ready(FirstReadyOutput { ready: out, error });
        }

        Poll::Pending
    }
}

pub(crate) fn observation_is_terminal(obs: &Observation) -> bool {
    !matches!(obs, Observation::Idle)
}

pub(crate) fn bound_observation_is_terminal<B>(value: &(B, Observation)) -> bool {
    observation_is_terminal(&value.1)
}

pub(crate) fn observation_is_replayable(obs: &Observation) -> bool {
    matches!(obs, Observation::Event(_))
}
