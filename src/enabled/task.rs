use std::{
    fmt,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{ready, Context, Poll},
};

use pin_project_lite::pin_project;
use tokio::{
    sync::{mpsc, oneshot},
    task::futures::TaskLocalFuture,
};

#[cfg(feature = "tracing")]
use tracing::{instrument::Instrumented, Instrument};

use crate::{enabled::operation::OperationMetadata, ParcheckLock};

pub fn task<F: Future>(name: &str, f: F) -> ParcheckTaskFuture<F> {
    ParcheckTaskFuture::Initial {
        data: Some((name, f)),
    }
}

pin_project! {
    #[doc(hidden)]
    #[project = ParcheckTaskFutureProj]
    pub enum ParcheckTaskFuture<'a, F> {
        Initial {
            data: Option<(&'a str, F)>,
        },
        Controlled {
            task: Task,

            #[pin]
            fut: InnerFuture<TaskLocalFuture<Task, F>>,
        },
        Uncontrolled {
            #[pin]
            fut: F,
        },
        Done,
    }

    impl<'a, F> PinnedDrop for ParcheckTaskFuture<'a, F> {
        fn drop(this: Pin<&mut Self>) {
            let proj = this.project();
            if let ParcheckTaskFutureProj::Controlled { task, .. } = proj {
                task.send_event(TaskEvent::TaskFinished);
            }
        }
    }
}

#[cfg(feature = "tracing")]
type InnerFuture<F> = Instrumented<F>;
#[cfg(not(feature = "tracing"))]
type InnerFuture<F> = F;

impl<'a, F: Future> Future for ParcheckTaskFuture<'a, F> {
    type Output = F::Output;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        loop {
            let this = self.as_mut().project();
            let new_state = match this {
                ParcheckTaskFutureProj::Initial { data } => {
                    let (name, fut) = data.take().unwrap();

                    if let Some(task) = Task::pop_expected_task(name) {
                        task.send_event(TaskEvent::TaskStarted);

                        let fut = TASK.scope(task.clone(), fut);
                        Self::Controlled {
                            #[cfg(feature = "tracing")]
                            fut: fut.instrument(tracing::info_span!(
                                "parcheck.task",
                                "parcheck.task.id" = task.id().0,
                                "parcheck.task.name" = name,
                            )),
                            #[cfg(not(feature = "tracing"))]
                            fut,
                            task,
                        }
                    } else {
                        Self::Uncontrolled { fut }
                    }
                }
                ParcheckTaskFutureProj::Controlled { task, fut } => {
                    let value = ready!(fut.poll(cx));
                    task.send_event(TaskEvent::TaskFinished);
                    self.set(Self::Done);
                    return Poll::Ready(value);
                }
                ParcheckTaskFutureProj::Uncontrolled { fut } => {
                    let value = ready!(fut.poll(cx));
                    self.set(Self::Done);
                    return Poll::Ready(value);
                }
                ParcheckTaskFutureProj::Done => panic!("future polled after done"),
            };
            self.set(new_state);
        }
    }
}

pub(crate) fn current() -> Option<Task> {
    TASK.try_with(|t| t.clone()).ok()
}

tokio::task_local! {
    static TASK: Task;
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) struct TaskId(pub(crate) usize);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskName(pub(crate) String);

#[derive(Clone)]
pub(crate) struct Task {
    inner: Arc<TaskInner>,
}

pub(crate) enum TaskEvent {
    TaskStarted,
    OperationPermitRequested {
        metadata: &'static OperationMetadata,
        permit: oneshot::Sender<OperationPermit>,
        locks: Vec<ParcheckLock>,
    },
    OperationFinished,
    TaskFinished,
}

#[derive(Debug)]
pub(crate) enum OperationPermit {
    Granted,
    OperationAlreadyInProgress { other: &'static OperationMetadata },
}

struct TaskInner {
    id: TaskId,
    name: TaskName,
    events: mpsc::UnboundedSender<(TaskId, TaskEvent)>,
}

impl fmt::Debug for Task {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Task")
            .field("id", &self.inner.id.0)
            .field("name", &self.inner.name.0)
            .finish()
    }
}

static EXPECTED_TASKS: Mutex<Vec<Task>> = Mutex::new(Vec::new());

impl Task {
    pub(crate) fn register(
        id: TaskId,
        name: TaskName,
        events: mpsc::UnboundedSender<(TaskId, TaskEvent)>,
    ) -> Self {
        let task = Self {
            inner: Arc::new(TaskInner { id, name, events }),
        };
        EXPECTED_TASKS.lock().unwrap().push(task.clone());
        task
    }

    pub(crate) fn send_event(&self, event: TaskEvent) {
        // ignore error
        let _ = self.inner.events.send((self.inner.id, event));
    }

    pub(crate) fn id(&self) -> TaskId {
        self.inner.id
    }

    pub(crate) fn name(&self) -> &TaskName {
        &self.inner.name
    }

    fn pop_expected_task(name: &str) -> Option<Task> {
        let mut expected = EXPECTED_TASKS.lock().unwrap();
        let idx = expected.iter().position(|task| task.inner.name.0 == name)?;
        Some(expected.swap_remove(idx))
    }
}
