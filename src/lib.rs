#![doc = include_str!("../README.md")]

#[cfg(feature = "enable")]
mod enabled;

#[cfg(not(feature = "enable"))]
mod disabled;

#[cfg(feature = "enable")]
#[doc(hidden)]
pub mod private {
    pub use super::enabled::{
        operation::{operation, OperationMetadata},
        task::task,
    };
}

#[cfg(feature = "enable")]
pub use enabled::runner::{runner, Runner, Trace};

#[derive(Clone, Debug)]
pub enum ParcheckLock {
    AcquireShared { scope: String },
    AcquireExclusive { scope: String },
    Release { scope: String },
}
