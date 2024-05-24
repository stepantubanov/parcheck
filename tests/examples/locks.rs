use std::sync::atomic::{AtomicBool, Ordering};

use parcheck::ParcheckLock;

static LOCK: AtomicBool = AtomicBool::new(false);

async fn execute(process: &str) {
    parcheck::task(&format!("locks:{process}"), async {
        parcheck::operation!(
            "acquire",
            [ParcheckLock::AcquireExclusive { scope: "".into() }],
            {
                async {
                    LOCK.compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
                        .expect("already locked, shouldn't execute this schedule");
                }
            }
        )
        .await;

        parcheck::operation!("locked", {
            async {
                assert!(LOCK.load(Ordering::Relaxed), "should be locked");
            }
        })
        .await;

        parcheck::operation!("release", [ParcheckLock::Release { scope: "".into() }], {
            async {
                LOCK.compare_exchange(true, false, Ordering::Relaxed, Ordering::Relaxed)
                    .expect("already unlocked, shouldn't be possible");
            }
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
