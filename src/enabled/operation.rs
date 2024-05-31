use std::{
    future::Future,
    pin::Pin,
    task::{ready, Context, Poll},
};

use futures_util::{future::FusedFuture, FutureExt};
use pin_project_lite::pin_project;
use tokio::sync::oneshot;

#[cfg(feature = "tracing")]
use tracing::{instrument::Instrumented, Instrument};

use crate::{
    enabled::task::{self, OperationPermit, Task},
    ParcheckLock,
};

#[doc(hidden)]
#[derive(Debug)]
pub struct OperationMetadata {
    pub name: &'static str,
    pub file: &'static str,
    pub line: u32,
}

#[doc(hidden)]
pub fn operation<F: Future>(
    metadata: &'static OperationMetadata,
    locks: Vec<ParcheckLock>,
    f: F,
) -> OperationFuture<F> {
    OperationFuture::Initial {
        data: Some((metadata, locks, f)),
    }
}

pin_project! {
    #[doc(hidden)]
    #[project = OperationFutureProj]
    pub enum OperationFuture<F> {
        Initial {
            data: Option<(&'static OperationMetadata, Vec<ParcheckLock>, F)>,
        },
        Uncontrolled {
            #[pin]
            fut: F,
        },
        WaitingForPermit {
            data: Option<(&'static OperationMetadata, Task, F)>,
            permit_rx: oneshot::Receiver<OperationPermit>,
        },
        Executing {
            task: Task,

            #[pin]
            fut: InnerFuture<F>,
        },
        Done,
    }
}

#[cfg(feature = "tracing")]
type InnerFuture<F> = Instrumented<F>;
#[cfg(not(feature = "tracing"))]
type InnerFuture<F> = F;

impl<F: Future> Future for OperationFuture<F> {
    type Output = F::Output;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        loop {
            let this = self.as_mut().project();
            let new_state = match this {
                OperationFutureProj::Initial { data } => {
                    // Can't fail because `Initial` state is only observed once
                    let (metadata, locks, fut) = data.take().unwrap();
                    match task::current() {
                        Some(task) => {
                            let (permit_tx, permit_rx) = oneshot::channel();
                            task.send_event(task::TaskEvent::OperationPermitRequested {
                                metadata,
                                permit: permit_tx,
                                locks,
                            });
                            // Can't fail because `Initial` state is only observed once
                            Self::WaitingForPermit {
                                permit_rx,
                                data: Some((metadata, task, fut)),
                            }
                        }
                        None => Self::Uncontrolled { fut },
                    }
                }
                OperationFutureProj::Uncontrolled { fut } => {
                    let value = ready!(fut.poll(cx));
                    self.set(Self::Done);
                    return Poll::Ready(value);
                }
                OperationFutureProj::WaitingForPermit { permit_rx, data } => {
                    let permit = ready!(permit_rx.poll_unpin(cx));
                    let (metadata, task, fut) = data.take().unwrap();

                    #[cfg(not(feature = "tracing"))]
                    let _ = metadata;

                    match permit {
                        Ok(OperationPermit::Granted) | Err(_) => Self::Executing {
                            task,
                            #[cfg(feature = "tracing")]
                            fut: fut.instrument(tracing::info_span!(
                                "parcheck.operation",
                                "parcheck.operation.name" = metadata.name,
                                "parcheck.file" = metadata.file,
                                "parcheck.line" = metadata.line
                            )),
                            #[cfg(not(feature = "tracing"))]
                            fut,
                        },
                        Ok(OperationPermit::OperationAlreadyInProgress { other }) => {
                            panic!(
                                "operation '{}' already in progress for task '{}' (operation at {}:{})",
                                other.name,
                                task.name().0,
                                other.file,
                                other.line
                            )
                        }
                    }
                }
                OperationFutureProj::Executing { task, fut } => {
                    let value = ready!(fut.poll(cx));
                    task.send_event(task::TaskEvent::OperationFinished);
                    self.set(Self::Done);
                    return Poll::Ready(value);
                }
                OperationFutureProj::Done => panic!("future polled after done"),
            };
            self.set(new_state);
        }
    }
}

impl<F: Future> FusedFuture for OperationFuture<F> {
    fn is_terminated(&self) -> bool {
        matches!(self, Self::Done)
    }
}
