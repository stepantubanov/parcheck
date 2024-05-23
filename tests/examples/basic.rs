use std::collections::HashMap;
use std::sync::Mutex;

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
        parcheck::task!(format!("execute:{process}"), async {
            parcheck::operation!(async {
                self.append(process);
            })
            .await;
            parcheck::operation!(async {
                self.append(process);
            })
            .await;
            parcheck::operation!(async {
                self.append(process);
            })
            .await;
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
            parcheck::task("one", async {}).await;
        })
        .await;
}

#[tokio::test]
async fn two_empty_tasks() {
    parcheck::runner()
        .run(["task_a", "task_b"], || async move {
            tokio::join!(
                parcheck::task("task_a", async {}),
                parcheck::task("task_b", async {}),
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
                parcheck::task("task_a", async {}),
                parcheck::task("task_b", async {}),
            );

            panic!("some kind of test failure");
        })
        .await;
}
