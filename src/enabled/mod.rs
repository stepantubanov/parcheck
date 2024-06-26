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
    ($name:expr, { $fut:expr }) => {
        $crate::private::task(&*$name, $fut)
    };
}

#[macro_export]
macro_rules! operation {
    ($name:literal, $locks:expr, { $fut:expr }) => {{
        static METADATA: $crate::private::OperationMetadata = $crate::private::OperationMetadata {
            name: $name,
            file: file!(),
            line: line!(),
        };
        $crate::private::operation(&METADATA, $locks, $fut)
    }};
    ($name:literal, {$fut:expr}) => {{
        static METADATA: $crate::private::OperationMetadata = $crate::private::OperationMetadata {
            name: $name,
            file: file!(),
            line: line!(),
        };
        $crate::private::operation(&METADATA, Vec::new(), $fut)
    }};
}
