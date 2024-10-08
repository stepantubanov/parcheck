use std::{collections::HashMap, fmt, mem::replace, time::Duration};

use tokio::{
    sync::{mpsc, oneshot},
    time::error::Elapsed,
};

use crate::{
    enabled::{
        operation::OperationMetadata,
        task::{OperationPermit, Task, TaskEvent, TaskId, TaskName},
    },
    ParcheckLock,
};

pub(crate) struct Controller {
    tasks: Vec<(Task, TaskState)>,
    locked_state: LockedState,
    // TODO: spawn tasks
    #[allow(dead_code)]
    events_tx: mpsc::UnboundedSender<(TaskId, TaskEvent)>,
    events_rx: mpsc::UnboundedReceiver<(TaskId, TaskEvent)>,
}

pub(crate) enum TaskState {
    NotStarted,
    ExecutingOutsideOperation,
    WaitingToStartOperation {
        metadata: &'static OperationMetadata,
        permit: oneshot::Sender<OperationPermit>,
        locks: Vec<ParcheckLock>,
        blocked_locks: Vec<ParcheckLock>,
    },
    ExecutingOperation {
        metadata: &'static OperationMetadata,
    },
    Finished,
    Invalid,
}

impl fmt::Display for OperationMetadata {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "'{}' (at {}:{})", self.name, self.file, self.line)
    }
}

impl fmt::Debug for TaskState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotStarted => f.write_str("not-started"),
            Self::ExecutingOutsideOperation => f.write_str("executing-outside-parcheck-operation"),
            Self::WaitingToStartOperation {
                metadata,
                locks,
                blocked_locks,
                ..
            } => {
                write!(f, "waiting-to-start-operation {metadata} (locks: {locks:?}, blocked by locks: {blocked_locks:?})")
            }
            Self::ExecutingOperation { metadata } => {
                write!(f, "executing-operation {metadata}")
            }
            Self::Finished => f.write_str("finished"),
            Self::Invalid => f.write_str("invalid"),
        }
    }
}

impl TaskState {
    pub(crate) fn can_execute(&self) -> bool {
        self.executable_op().is_some()
    }

    pub(crate) fn executable_op(&self) -> Option<&'static OperationMetadata> {
        match self {
            Self::WaitingToStartOperation {
                metadata,
                blocked_locks,
                ..
            } if blocked_locks.is_empty() => Some(metadata),
            _ => None,
        }
    }
}

impl Controller {
    pub(crate) fn register(initial_tasks: &[TaskName]) -> Self {
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let tasks = initial_tasks
            .iter()
            .enumerate()
            .map(|(i, name)| {
                (
                    Task::register(TaskId(i), name.clone(), events_tx.clone()),
                    TaskState::NotStarted,
                )
            })
            .collect();

        Self {
            tasks,
            locked_state: LockedState::new(),
            events_tx,
            events_rx,
        }
    }

    pub(crate) async fn ready(&mut self, timeout: Duration) -> &[(Task, TaskState)] {
        let this = &mut *self;
        let result = tokio::time::timeout(timeout, async move {
            loop {
                if this.tasks.iter().all(|(_, state)| {
                    matches!(
                        state,
                        TaskState::WaitingToStartOperation { .. } | TaskState::Finished
                    )
                }) {
                    for (task, state) in &mut this.tasks {
                        let TaskState::WaitingToStartOperation {
                            locks,
                            blocked_locks,
                            ..
                        } = state
                        else {
                            continue;
                        };
                        *blocked_locks = this.locked_state.blocked(task.id(), locks);
                    }
                    break;
                }

                this.recv_event().await;
            }
        })
        .await;

        match result {
            Ok(()) => &self.tasks,
            Err(Elapsed { .. }) => {
                let tasks = self
                    .tasks
                    .iter()
                    .map(|(task, state)| format!("task '{}': {state:?}", task.name().0))
                    .collect::<Vec<_>>();
                panic!("timed out, tasks: {tasks:#?}");
            }
        }
    }

    pub(crate) async fn step_forward(&mut self, id: TaskId) {
        let (_, state) = &mut self.tasks[id.0];

        let prev = replace(state, TaskState::Invalid);
        let TaskState::WaitingToStartOperation {
            metadata,
            permit,
            locks,
            blocked_locks,
        } = prev
        else {
            panic!("step_forward: task not waiting: {prev:?}");
        };
        *state = TaskState::ExecutingOperation { metadata };

        assert!(
            blocked_locks.is_empty(),
            "step_forward: blocked by locks: {blocked_locks:?}"
        );
        self.locked_state.acquire_locks(id, &locks);

        // ignore error (channel closed)
        let _ = permit.send(OperationPermit::Granted);

        while matches!(self.tasks[id.0], (_, TaskState::ExecutingOperation { .. })) {
            self.recv_event().await;
        }

        self.locked_state.release_locks(id, &locks);
    }

    pub(crate) fn tasks(&self) -> &[(Task, TaskState)] {
        &self.tasks
    }

    pub(crate) fn assert_finished(&self) {
        let unfinished = self
            .tasks
            .iter()
            .filter_map(|(task, state)| {
                if matches!(state, TaskState::Finished) {
                    None
                } else {
                    Some(format!("task '{}': {state:?}", task.name().0))
                }
            })
            .collect::<Vec<_>>();

        assert!(
            unfinished.is_empty(),
            "some tasks did not finish: {unfinished:#?}"
        );
    }

    async fn recv_event(&mut self) {
        // Channel can't be closed here because controller keeps a sender too.
        let (id, event) = self.events_rx.recv().await.expect("channel closed");
        let (task, state) = &mut self.tasks[id.0];
        *state = match event {
            TaskEvent::TaskStarted => TaskState::ExecutingOutsideOperation,
            TaskEvent::OperationPermitRequested {
                metadata,
                permit,
                locks,
            } => {
                if let TaskState::ExecutingOperation { metadata: other } = state {
                    let _ = permit
                        .send(super::task::OperationPermit::OperationAlreadyInProgress { other });
                    return;
                };

                TaskState::WaitingToStartOperation {
                    metadata,
                    permit,
                    blocked_locks: Vec::new(),
                    locks,
                }
            }
            TaskEvent::OperationFinished => {
                let TaskState::ExecutingOperation { .. } = state else {
                    panic!(
                        "task '{}': received OperationFinished when not inside operation",
                        task.name().0
                    );
                };

                TaskState::ExecutingOutsideOperation
            }
            TaskEvent::TaskFinished => {
                let locks = self.locked_state.acquired_locks(id);
                assert!(
                    locks.is_empty(),
                    "task '{}': finished without releasing locks: {locks:?}",
                    task.name().0
                );
                TaskState::Finished
            }
        };
    }
}

#[derive(Debug)]
struct LockedState {
    scopes: HashMap<String, Vec<(TaskId, Mode)>>,
}

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
enum Mode {
    Shared,
    Exclusive,
}

impl LockedState {
    fn new() -> Self {
        Self {
            scopes: HashMap::default(),
        }
    }

    fn blocked(&self, task_id: TaskId, locks: &[ParcheckLock]) -> Vec<ParcheckLock> {
        let mut blockers = Vec::new();
        for lock in locks {
            let (scope, mode) = match lock {
                ParcheckLock::AcquireShared { scope } => (scope, Mode::Shared),
                ParcheckLock::AcquireExclusive { scope } => (scope, Mode::Exclusive),
                ParcheckLock::Release { .. } => continue,
            };
            if let Some(holders) = self.scopes.get(scope) {
                if has_conflict(task_id, mode, holders) {
                    blockers.push(lock.clone());
                }
            }
        }

        blockers
    }

    fn acquire_locks(&mut self, task_id: TaskId, locks: &[ParcheckLock]) {
        for lock in locks {
            let (scope, mode) = match lock {
                ParcheckLock::AcquireShared { scope } => (scope, Mode::Shared),
                ParcheckLock::AcquireExclusive { scope } => (scope, Mode::Exclusive),
                ParcheckLock::Release { .. } => continue,
            };

            let holders = self.scopes.entry(scope.clone()).or_default();
            assert!(
                !has_conflict(task_id, mode, holders),
                "acquire_locks() acquire lock conflict on {scope}"
            );

            if let Some((_, holder_mode)) = holders
                .iter_mut()
                .find(|(holder_task_id, _)| *holder_task_id == task_id)
            {
                if mode == Mode::Exclusive {
                    *holder_mode = mode;
                }
            } else {
                holders.push((task_id, mode));
            }
        }
    }

    fn release_locks(&mut self, task_id: TaskId, locks: &[ParcheckLock]) {
        for lock in locks {
            let scope = match lock {
                ParcheckLock::AcquireShared { .. } | ParcheckLock::AcquireExclusive { .. } => {
                    continue
                }
                ParcheckLock::Release { scope } => scope,
            };

            let Some(holders) = self.scopes.get_mut(scope) else {
                continue;
            };
            if let Some(idx) = holders
                .iter()
                .position(|(holder_task_id, _)| *holder_task_id == task_id)
            {
                holders.swap_remove(idx);
            }
        }
    }

    fn acquired_locks(&self, task_id: TaskId) -> Vec<String> {
        self.scopes
            .iter()
            .filter(move |(_, holders)| {
                holders
                    .iter()
                    .any(|(holder_task_id, _)| *holder_task_id == task_id)
            })
            .map(|(scope, _)| scope.clone())
            .collect()
    }
}

fn has_conflict(task_id: TaskId, mode: Mode, holders: &[(TaskId, Mode)]) -> bool {
    for (holder_task_id, holder_mode) in holders {
        if *holder_task_id != task_id
            && (*holder_mode == Mode::Exclusive || mode == Mode::Exclusive)
        {
            return true;
        }
    }
    false
}
