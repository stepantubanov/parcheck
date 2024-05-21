#[cfg(feature = "enable")]
mod enabled;
#[cfg(feature = "enable")]
use enabled as api;

#[cfg(not(feature = "enable"))]
mod disabled;
#[cfg(not(feature = "enable"))]
use disabled as api;

#[derive(Clone, Debug)]
pub enum Lock {
    AcquireShared { scope: String },
    AcquireExclusive { scope: String },
    Release { scope: String },
}

pub use api::operation::{operation, operation_with_lock};
pub use api::task::task;

#[cfg(feature = "enable")]
pub use enabled::runner::runner;

#[cfg(feature = "enable")]
#[macro_export]
macro_rules! cfg_if {
    ($code:block) => {
        $code
    };
}

#[cfg(not(feature = "enable"))]
#[macro_export]
macro_rules! cfg_if {
    ($code:block) => {};
}
