use std::future::Future;

use tokio::sync::oneshot;

use crate::{
    enabled::task::{self, OperationPermit},
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
pub async fn operation<F: Future>(
    metadata: &'static OperationMetadata,
    locks: Vec<ParcheckLock>,
    f: F,
) -> F::Output {
    let Some(task) = task::current() else {
        return f.await;
    };

    let (permit_tx, permit_rx) = oneshot::channel();
    task.send_event(task::TaskEvent::OperationPermitRequested {
        metadata,
        permit: permit_tx,
        locks,
    });
    match permit_rx.await.unwrap_or(OperationPermit::Granted) {
        OperationPermit::Granted => {}
        OperationPermit::OperationAlreadyInProgress { other } => {
            panic!(
                "operation '{}' already in progress for task '{}' (operation at {}:{})",
                other.name,
                task.name().0,
                other.file,
                other.line
            )
        }
    }

    #[cfg(not(feature = "tracing"))]
    let value = f.await;
    #[cfg(feature = "tracing")]
    let value = {
        use tracing::instrument::Instrument;
        f.instrument(tracing::info_span!(
            "parcheck.operation",
            "parcheck.operation.name" = metadata.name,
            "parcheck.file" = metadata.file,
            "parcheck.line" = metadata.line
        ))
        .await
    };

    task.send_event(task::TaskEvent::OperationFinished);
    value
}
