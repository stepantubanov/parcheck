#[cfg(feature = "enable")]
mod enabled;
#[cfg(feature = "enable")]
use enabled as api;

#[cfg(not(feature = "enable"))]
mod disabled;
#[cfg(not(feature = "enable"))]
use disabled as api;

#[doc(hidden)]
pub mod private {
    pub use super::api::{
        operation::{operation, OperationMetadata},
        task::task,
    };
}

pub use api::task::task;

#[cfg(feature = "enable")]
pub use enabled::runner::runner;

#[derive(Clone, Debug)]
pub enum ParcheckLock {
    AcquireShared { scope: String },
    AcquireExclusive { scope: String },
    Release { scope: String },
}
