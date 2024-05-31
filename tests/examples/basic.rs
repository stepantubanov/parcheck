use std::collections::HashMap;
use std::future;
use std::mem::size_of_val;
use std::sync::Mutex;

use parcheck::Trace;

struct Observer {
    trace: Mutex<String>,
}

impl Observer {
    fn new() -> Self {
        Self {
            trace: Mutex::new(String::with_capacity(128)),
        }
    }

    fn take_trace(&self) -> String {
        std::mem::take(&mut self.trace.lock().unwrap())
    }

    async fn execute(&self, process: &str) {
        parcheck::task!(format!("execute:{process}"), {
            async {
                parcheck::operation!("append:1", {
                    async {
                        self.append(process);
                    }
                })
                .await;
                parcheck::operation!("append:2", {
                    async {
                        self.append(process);
                    }
                })
                .await;
                parcheck::operation!("append:3", {
                    async {
                        self.append(process);
                    }
                })
                .await;
            }
        })
        .await;
    }

    fn append(&self, process: &str) {
        self.trace.lock().unwrap().push_str(process);
    }
}

#[tokio::test]
async fn covers_different_linearizations() {
    macro_rules! assert_has_key {
        ($v:ident, $key:literal) => {{
            assert!($v.contains_key($key), "does not contain '{}'", $key);
        }};
    }

    // Total number of schedules in this example is (n + m)! / (n!m!) = (3 + 3)! / 3! / 3! = 20

    let mut traces: HashMap<String, usize> = HashMap::with_capacity(32);

    parcheck::runner()
        .run_with_state(
            ["execute:a", "execute:b"],
            &mut traces,
            |traces| async move {
                let obs = Observer::new();
                tokio::join!(obs.execute("a"), obs.execute("b"));

                let count = traces.entry(obs.take_trace()).or_default();
                *count += 1;

                traces
            },
        )
        .await;

    println!("traces: {traces:?}",);

    assert_has_key!(traces, "aaabbb");
    assert_has_key!(traces, "bbbaaa");

    assert_has_key!(traces, "ababab");
    assert_has_key!(traces, "aabbab");
    assert_has_key!(traces, "bababa");
    assert_has_key!(traces, "bbaaba");
    assert_has_key!(traces, "aababb");
    assert_has_key!(traces, "bbabaa");
}

#[tokio::test]
async fn no_tasks() {
    let initial: [&str; 0] = [];
    parcheck::runner().run(initial, || async {}).await;
}

#[tokio::test]
async fn one_empty_task() {
    parcheck::runner()
        .run(["one"], || async move {
            parcheck::task!("one", { async {} }).await;
        })
        .await;
}

#[tokio::test]
async fn two_empty_tasks() {
    parcheck::runner()
        .run(["task_a", "task_b"], || async move {
            tokio::join!(
                parcheck::task!("task_a", { async {} }),
                parcheck::task!("task_b", { async {} }),
            );
        })
        .await;
}

#[tokio::test]
#[should_panic(expected = "some kind of test failure")]
async fn does_not_double_panic() {
    parcheck::runner()
        .run(["task_a", "task_b"], || async move {
            tokio::join!(
                parcheck::task!("task_a", { async {} }),
                parcheck::task!("task_b", { async {} }),
            );

            panic!("some kind of test failure");
        })
        .await;
}

#[tokio::test]
#[should_panic(
    expected = "operation 'outer' already in progress for task 'reentrant' (operation at tests/examples/basic.rs:146)"
)]
async fn detects_reentrant_task() {
    parcheck::runner()
        .run(["reentrant"], || async move {
            parcheck::task!("reentrant", {
                async {
                    parcheck::operation!("outer", {
                        async {
                            parcheck::operation!("inner", { async {} }).await;
                        }
                    })
                    .await;
                }
            })
            .await;
        })
        .await;
}

#[tokio::test]
async fn replays_prefix_trace() {
    // only a prefix of an actual execution
    let trace: Trace = "0:replays_prefix_trace.op > 0:replays_prefix_trace.op"
        .parse()
        .unwrap();

    parcheck::runner()
        .replay(trace)
        .run(["replays_prefix_trace"], || async {
            parcheck::task!("replays_prefix_trace", {
                async {
                    for _ in 0..3 {
                        parcheck::operation!("op", { async {} }).await;
                    }
                }
            })
            .await;
        })
        .await;
}

#[tokio::test]
async fn replays_full_trace() {
    let trace: Trace =
        "0:replays_full_trace.op > 0:replays_full_trace.op > 0:replays_full_trace.op"
            .parse()
            .unwrap();

    parcheck::runner()
        .replay(trace)
        .run(["replays_full_trace"], || async {
            parcheck::task!("replays_full_trace", {
                async {
                    for _ in 0..3 {
                        parcheck::operation!("op", { async {} }).await;
                    }
                }
            })
            .await;
        })
        .await;
}

#[tokio::test]
async fn futures_arent_too_large() {
    let operation = future::ready([0_u8; 500]);
    let operation_size = size_of_val(&operation);
    let wrapped_operation_size =
        size_of_val(&parcheck::operation!("operation_size", { operation }));
    assert!(
        wrapped_operation_size - operation_size < 64,
        "op future size: {operation_size} -> {wrapped_operation_size}"
    );

    let task = future::ready([0_u8; 500]);
    let task_size = size_of_val(&task);
    let wrapped_task_size = size_of_val(&parcheck::task!("task_size", { task }));
    assert!(
        wrapped_task_size - task_size < 96,
        "task future size: {task_size} -> {wrapped_task_size}"
    );
}
