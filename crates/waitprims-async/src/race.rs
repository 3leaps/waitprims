//! First-ready collection across observer arms.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::observer::Observation;
use waitprims_core::Result;

enum Arm<F> {
    Pending(Pin<Box<F>>),
    Ready(Observation),
}

/// Wait until at least one arm yields a terminal observation, or every arm
/// yields [`Observation::Idle`].
pub(crate) struct ObservationRace<F> {
    arms: Vec<Arm<F>>,
}

impl<F> ObservationRace<F>
where
    F: Future<Output = Result<Observation>>,
{
    pub(crate) fn new(futures: impl IntoIterator<Item = F>) -> Self {
        Self {
            arms: futures
                .into_iter()
                .map(|fut| Arm::Pending(Box::pin(fut)))
                .collect(),
        }
    }
}

impl<F> Future for ObservationRace<F>
where
    F: Future<Output = Result<Observation>>,
{
    type Output = Result<Vec<(usize, Observation)>>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let mut pending = false;
        let mut terminal = false;

        for arm in &mut this.arms {
            if let Arm::Pending(fut) = arm {
                match fut.as_mut().poll(cx) {
                    Poll::Ready(Ok(obs)) => {
                        if !matches!(obs, Observation::Idle) {
                            terminal = true;
                        }
                        *arm = Arm::Ready(obs);
                    }
                    Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
                    Poll::Pending => pending = true,
                }
            } else if let Arm::Ready(obs) = arm {
                if !matches!(obs, Observation::Idle) {
                    terminal = true;
                }
            }
        }

        if terminal {
            let mut out = Vec::new();
            for (idx, arm) in this.arms.iter_mut().enumerate() {
                match arm {
                    Arm::Ready(obs) => out.push((idx, obs.clone())),
                    Arm::Pending(fut) => match fut.as_mut().poll(cx) {
                        Poll::Ready(Ok(obs)) => out.push((idx, obs)),
                        Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
                        Poll::Pending => {}
                    },
                }
            }
            return Poll::Ready(Ok(out));
        }

        if !pending {
            let out = this
                .arms
                .iter()
                .enumerate()
                .filter_map(|(idx, arm)| match arm {
                    Arm::Ready(obs) => Some((idx, obs.clone())),
                    Arm::Pending(_) => None,
                })
                .collect();
            return Poll::Ready(Ok(out));
        }

        Poll::Pending
    }
}
