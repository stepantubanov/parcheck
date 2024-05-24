use parcheck::ParcheckLock;

#[tokio::test]
async fn works_when_disabled() {
    let result = parcheck::task!("task", async {
        parcheck::operation!("op", { async { 123 } }).await
    })
    .await;
    assert_eq!(result, 123);

    let result = parcheck::task("task", async {
        parcheck::operation!("op", { async { 123 } }).await
    })
    .await;
    assert_eq!(result, 123);
}

#[tokio::test]
async fn doesnt_complain_about_unused_vars() {
    let x = 123;
    let result = parcheck::task!(format!("task:{x}"), async {
        parcheck::operation!(
            "op",
            [ParcheckLock::AcquireExclusive {
                scope: "scope".into()
            }],
            { async { 123 } }
        )
        .await
    })
    .await;
    assert_eq!(result, 123);

    let result = parcheck::task("task", async {
        parcheck::operation!(
            "op",
            [ParcheckLock::AcquireExclusive {
                scope: "scope".into()
            }],
            { async { 123 } }
        )
        .await
    })
    .await;
    assert_eq!(result, 123);
}
