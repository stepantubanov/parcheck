#[macro_export]
macro_rules! cfg_if {
    ($code:block) => {};
}

#[macro_export]
macro_rules! task {
    ($name:expr, $fut:expr) => {{
        let _ = $name;
        $fut
    }};
}

#[macro_export]
macro_rules! operation {
    ($name:literal, $locks:expr, { $fut:expr }) => {{
        {
            let _ = || $locks;
        }
        $fut
    }};
    ($name:literal, { $fut:expr }) => {{
        $fut
    }};
}
