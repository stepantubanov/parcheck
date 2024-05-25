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

#[tokio::test]
#[should_panic(
    expected = "task 'unreleased_locks': finished without releasing locks: [\"lock-scope\"]"
)]
async fn panics_if_finished_without_releasing_locks() {
    parcheck::runner()
        .run(["unreleased_locks"], || async {
            parcheck::task("unreleased_locks", async {
                parcheck::operation!(
                    "acquire",
                    [ParcheckLock::AcquireExclusive {
                        scope: "lock-scope".into()
                    }],
                    { async {} }
                )
                .await;
            })
            .await;
        })
        .await;
}

#[tokio::test]
#[should_panic(expected = "some tasks did not finish")]
async fn detects_deadlocks() {
    async fn execute(name: &str, lock_a: &str, lock_b: &str) {
        parcheck::task(name, async {
            parcheck::operation!(
                "acquire",
                [ParcheckLock::AcquireExclusive {
                    scope: lock_a.into()
                }],
                { async {} }
            )
            .await;

            parcheck::operation!(
                "acquire",
                [ParcheckLock::AcquireExclusive {
                    scope: lock_b.into()
                }],
                { async {} }
            )
            .await;

            parcheck::operation!(
                "release",
                [ParcheckLock::Release {
                    scope: lock_b.into()
                }],
                { async {} }
            )
            .await;

            parcheck::operation!(
                "release",
                [ParcheckLock::Release {
                    scope: lock_a.into()
                }],
                { async {} }
            )
            .await;
        })
        .await;
    }

    parcheck::runner()
        .run(["deadlock:a", "deadlock:b"], || async {
            tokio::join!(
                execute("deadlock:a", "0", "1"),
                execute("deadlock:b", "1", "0"),
            );
        })
        .await;
}
