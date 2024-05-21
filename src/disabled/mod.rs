pub(crate) mod operation {
    use crate::Lock;
    use std::future::Future;

    #[inline(always)]
    pub fn operation<F: Future>(f: F) -> F {
        f
    }

    #[inline(always)]
    pub fn operation_with_lock<F: Future>(_lock: Lock, f: F) -> F {
        f
    }
}

pub(crate) mod task {
    use std::future::Future;

    #[inline(always)]
    pub fn task<F: Future>(_name: &str, f: F) -> F {
        f
    }
}
