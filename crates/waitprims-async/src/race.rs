//! First-ready collection across observer arms.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::observer::Observation;
use waitprims_core::Result;

enum Arm<F, T> {
    Pending(Pin<Box<F>>),
    Ready(Option<T>),
}

/// Wait until at least one arm yields a terminal value, or every arm is ready
/// with a non-terminal value (all idle).
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
}

impl<F, T> Future for FirstReady<F, T>
where
    F: Future<Output = Result<T>>,
    T: Unpin,
{
    type Output = Result<Vec<(usize, T)>>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let mut pending = false;
        let mut terminal = false;

        for arm in &mut this.arms {
            if let Arm::Pending(fut) = arm {
                match fut.as_mut().poll(cx) {
                    Poll::Ready(Ok(value)) => {
                        if (this.is_terminal)(&value) {
                            terminal = true;
                        }
                        *arm = Arm::Ready(Some(value));
                    }
                    Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
                    Poll::Pending => pending = true,
                }
            } else if let Arm::Ready(Some(value)) = arm {
                if (this.is_terminal)(value) {
                    terminal = true;
                }
            }
        }

        if terminal || !pending {
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
                        Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
                        Poll::Pending => {}
                    },
                }
            }
            return Poll::Ready(Ok(out));
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
