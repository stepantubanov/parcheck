use std::future::Future;

use tokio::sync::oneshot;

use crate::{
    enabled::task::{self, OperationPermit},
    ParcheckLock,
};

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
    match permit_rx.await.unwrap_or(OperationPermit::Granted) {
        OperationPermit::Granted => {}
        OperationPermit::OperationAlreadyInProgress => {
            panic!("operation already in progress for task '{}'", task.name().0)
        }
    }

    let value = f.await;

    task.send_event(task::TaskEvent::OperationFinished).await;
    value
}
