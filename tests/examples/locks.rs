use std::sync::atomic::{AtomicBool, Ordering};

use parcheck::Lock;

static LOCK: AtomicBool = AtomicBool::new(false);

async fn execute(process: &str) {
    parcheck::task(&format!("locks:{process}"), async {
        parcheck::operation_with_lock(Lock::AcquireExclusive { scope: "".into() }, async {
            LOCK.compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
                .expect("already locked, shouldn't execute this schedule");
        })
        .await;

        parcheck::operation(async {
            assert!(LOCK.load(Ordering::Relaxed), "should be locked");
        })
        .await;

        parcheck::operation_with_lock(Lock::Release { scope: "".into() }, async {
            LOCK.compare_exchange(true, false, Ordering::Relaxed, Ordering::Relaxed)
                .expect("already unlocked, shouldn't be possible");
        })
        .await;
    })
    .await;
}

#[tokio::test]
async fn respects_locks() {
    parcheck::runner()
        .run(["locks:a", "locks:b"], || async {
            tokio::join!(execute("a"), execute("b"));
        })
        .await;
}
