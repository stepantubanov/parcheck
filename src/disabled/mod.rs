pub(crate) mod task {
    use std::future::Future;

    #[inline(always)]
    pub fn task<F: Future>(_name: &str, f: F) -> F {
        f
    }
}

pub(crate) mod operation {
    use crate::ParcheckLock;
    use std::future::Future;

    #[inline(always)]
    pub async fn operation<F: Future>(_locks: Vec<ParcheckLock>, f: F) -> F::Output {
        f.await
    }
}

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
    ([$($lock:expr),+], { $fut:expr }) => {{
        {
            let _ = || [$($lock),+];
        }
        $fut
    }};
    ({ $fut:expr }) => {{
        $fut
    }};
}
