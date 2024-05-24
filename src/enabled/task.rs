use std::{
    fmt,
    future::Future,
    sync::{Arc, Mutex},
};

use tokio::sync::{mpsc, oneshot};

use crate::{enabled::operation::OperationMetadata, ParcheckLock};

pub async fn task<F: Future>(name: &str, f: F) -> F::Output {
    if let Some(task) = Task::pop_expected_task(name) {
        task.send_event(TaskEvent::TaskStarted).await;

        #[cfg(not(feature = "tracing"))]
        let value = TASK.scope(task.clone(), f).await;

        #[cfg(feature = "tracing")]
        let value = {
            use tracing::instrument::Instrument;
            TASK.scope(task.clone(), f)
                .instrument(tracing::info_span!(
                    "parcheck.task",
                    "parcheck.task.id" = task.id().0,
                    "parcheck.task.name" = name,
                ))
                .await
        };

        task.send_event(TaskEvent::TaskFinished).await;
        value
    } else {
        f.await
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
    events: mpsc::Sender<(TaskId, TaskEvent)>,
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
        events: mpsc::Sender<(TaskId, TaskEvent)>,
    ) -> Self {
        let task = Self {
            inner: Arc::new(TaskInner { id, name, events }),
        };
        EXPECTED_TASKS.lock().unwrap().push(task.clone());
        task
    }

    pub(crate) async fn send_event(&self, event: TaskEvent) {
        // ignore error
        let _ = self.inner.events.send((self.inner.id, event)).await;
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
