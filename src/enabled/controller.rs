use std::{collections::HashMap, mem::replace};

use tokio::sync::{mpsc, oneshot};

use crate::{
    enabled::task::{Task, TaskEvent, TaskId, TaskName},
    Lock,
};

pub(crate) struct Controller {
    tasks: Vec<(Task, TaskState)>,
    locked_state: LockedState,
    // TODO: spawn tasks
    #[allow(dead_code)]
    events_tx: mpsc::Sender<(TaskId, TaskEvent)>,
    events_rx: mpsc::Receiver<(TaskId, TaskEvent)>,
}

#[derive(Debug)]
pub(crate) enum TaskState {
    DidNotStart,
    OutsideOperation,
    WaitingForPermit {
        permit: oneshot::Sender<()>,
        locks: Vec<Lock>,
        blocked_locks: Vec<Lock>,
    },
    InsideOperation,
    Finished,
}

impl Controller {
    pub(crate) fn register(initial_tasks: &[TaskName]) -> Self {
        let (events_tx, events_rx) = mpsc::channel(32);
        let tasks = initial_tasks
            .iter()
            .enumerate()
            .map(|(i, name)| {
                (
                    Task::register(TaskId(i), name.clone(), events_tx.clone()),
                    TaskState::DidNotStart,
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

    pub(crate) async fn ready(&mut self) -> &[(Task, TaskState)] {
        // TODO: timeout

        loop {
            if self.tasks.iter().all(|(_, state)| {
                matches!(
                    state,
                    TaskState::WaitingForPermit { .. } | TaskState::Finished
                )
            }) {
                for (task, state) in &mut self.tasks {
                    let TaskState::WaitingForPermit {
                        locks,
                        blocked_locks,
                        ..
                    } = state
                    else {
                        continue;
                    };
                    *blocked_locks = self.locked_state.blocked(task.id(), locks);
                }
                return &self.tasks;
            }

            self.recv_event().await;
        }
    }

    pub(crate) async fn step_forward(&mut self, id: TaskId) {
        let (_, state) = &mut self.tasks[id.0];

        let prev = replace(state, TaskState::InsideOperation);
        let TaskState::WaitingForPermit {
            permit,
            locks,
            blocked_locks,
        } = prev
        else {
            panic!("step_forward: task not waiting: {prev:?}");
        };

        assert!(
            blocked_locks.is_empty(),
            "step_forward: blocked by locks: {blocked_locks:?}"
        );
        self.locked_state.acquire_locks(id, &locks);

        // ignore error (channel closed)
        let _ = permit.send(());

        while matches!(self.tasks[id.0], (_, TaskState::InsideOperation)) {
            self.recv_event().await;
        }

        self.locked_state.release_locks(id, &locks);
    }

    async fn recv_event(&mut self) {
        // Channel can't be closed here because controller keeps a sender too.
        let (id, event) = self.events_rx.recv().await.expect("channel closed");
        let (_, state) = &mut self.tasks[id.0];
        *state = match event {
            TaskEvent::TaskStarted => TaskState::OutsideOperation,
            TaskEvent::OperationPermitRequested { permit, locks } => TaskState::WaitingForPermit {
                permit,
                blocked_locks: Vec::new(),
                locks,
            },
            TaskEvent::OperationFinished => {
                let TaskState::InsideOperation = state else {
                    panic!("received OperationFinished when not inside operation");
                };

                TaskState::OutsideOperation
            }
            TaskEvent::TaskFinished => TaskState::Finished,
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

    fn blocked(&self, task_id: TaskId, locks: &[Lock]) -> Vec<Lock> {
        let mut blockers = Vec::new();
        for lock in locks {
            let (scope, mode) = match lock {
                Lock::AcquireShared { scope } => (scope, Mode::Shared),
                Lock::AcquireExclusive { scope } => (scope, Mode::Exclusive),
                Lock::Release { .. } => continue,
            };
            if let Some(holders) = self.scopes.get(scope) {
                if has_conflict(task_id, mode, holders) {
                    blockers.push(lock.clone());
                }
            }
        }

        blockers
    }

    fn acquire_locks(&mut self, task_id: TaskId, locks: &[Lock]) {
        for lock in locks {
            let (scope, mode) = match lock {
                Lock::AcquireShared { scope } => (scope, Mode::Shared),
                Lock::AcquireExclusive { scope } => (scope, Mode::Exclusive),
                Lock::Release { .. } => continue,
            };

            let holders = self.scopes.entry(scope.clone()).or_default();
            if has_conflict(task_id, mode, holders) {
                panic!("acquire_locks() acquire lock conflict on {scope}");
            };

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

    fn release_locks(&mut self, task_id: TaskId, locks: &[Lock]) {
        for lock in locks {
            let scope = match lock {
                Lock::AcquireShared { .. } | Lock::AcquireExclusive { .. } => continue,
                Lock::Release { scope } => scope,
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
