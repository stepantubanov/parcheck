pub(crate) mod controller;
pub(crate) mod operation;
pub(crate) mod runner;
pub(crate) mod schedule_tree;
pub(crate) mod task;

#[macro_export]
macro_rules! cfg_if {
    ($code:block) => {
        $code
    };
}

#[macro_export]
macro_rules! task {
    ($name:expr, $fut:expr) => {
        $crate::task(&*$name, $fut)
    };
}

#[cfg(not(feature = "tracing"))]
#[macro_export]
macro_rules! operation {
    ([$($lock:expr),+], $fut:expr) => {
        $crate::operation(vec![$($lock),+], $fut)
    };
    ($fut:expr) => {
        $crate::operation(Vec::new(), $fut)
    };
}

#[cfg(feature = "tracing")]
#[macro_export]
macro_rules! operation {
    ([$($lock:expr),+], $fut:expr) => {
        {
            use tracing::instrument::Instrument as _;
            $crate::operation(vec![$($lock),+], $fut).instrument(tracing::info_span!("parcheck.operation"))
        }
    };
    ($fut:expr) => {
        {
            use tracing::instrument::Instrument as _;
            $crate::operation(Vec::new(), $fut).instrument(tracing::info_span!("parcheck.operation"))
        }
    };
}
