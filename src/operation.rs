use std::future::Future;

use tokio::sync::oneshot;

use crate::task::{self, Task};

pub async fn operation<F: Future>(f: F) -> F::Output {
    let Some(task) = task::current() else {
        return f.await;
    };

    acquire_permit(&task, Vec::new()).await;

    let value = f.await;

    task.send_event(task::TaskEvent::OperationFinished).await;
    value
}

#[derive(Clone, Debug)]
pub enum Lock {
    AcquireShared { scope: String },
    AcquireExclusive { scope: String },
    Release { scope: String },
}

pub async fn operation_with_lock<F: Future>(lock: Lock, f: F) -> F::Output {
    let Some(task) = task::current() else {
        return f.await;
    };

    acquire_permit(&task, vec![lock]).await;

    let value = f.await;

    task.send_event(task::TaskEvent::OperationFinished).await;
    value
}

async fn acquire_permit(task: &Task, locks: Vec<Lock>) {
    let (permit_tx, permit_rx) = oneshot::channel();

    task.send_event(task::TaskEvent::OperationPermitRequested {
        permit: permit_tx,
        locks,
    })
    .await;
    // ignore error
    let _ = permit_rx.await;
}
