use std::future::Future;

use tokio::sync::oneshot;

use crate::{enabled::task, ParcheckLock};

pub async fn operation<F: Future>(locks: Vec<ParcheckLock>, f: F) -> F::Output {
    let Some(task) = task::current() else {
        return f.await;
    };

    let (permit_tx, permit_rx) = oneshot::channel();
    task.send_event(task::TaskEvent::OperationPermitRequested {
        permit: permit_tx,
        locks,
    })
    .await;
    let _ = permit_rx.await;

    let value = f.await;

    task.send_event(task::TaskEvent::OperationFinished).await;
    value
}
