# parcheck

`parcheck` is a concurrency testing tool for Rust. It helps uncover bugs by running testing
scenarios with different thread/task interleavings.

[![Crates.io](https://img.shields.io/crates/v/parcheck.svg)](https://crates.io/crates/parcheck)
[![Documentation](https://img.shields.io/docsrs/parcheck/latest)](https://docs.rs/parcheck/latest/parcheck/)
[![Build status](https://github.com/stepantubanov/parcheck/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/stepantubanov/parcheck/actions/workflows/ci.yml)

## Usage

Currently in **alpha** version, interface will change soon, but the idea remains the same.

Add it to your dependencies:

```toml
# Cargo.toml
[dependencies]
parcheck = { version = "0.1.0-alpha.2", git = "https://github.com/stepantubanov/parcheck.git" }

[dev-dependencies]
parcheck = { version = "0.1.0-alpha.2", features = ["enable"], git = "https://github.com/stepantubanov/parcheck.git" }
```

Note that `enable` feature is enabled **only** in `dev-dependecies`. Without this feature calls to
`parcheck::task` and `parcheck::operation` expand to underlying future without any additional logic,
so effect on production code should be as minimal as possible.

In order for the code to be controlled, it has to be instrumented with calls to `parcheck::task` and
`parcheck::operation`. "Tasks" are threads of execution (currently `parcheck` does not support
intra-task concurrency, but that's coming soon) and "operations" are futures that `parcheck` will
control. Specifically it'll execute operations in a deterministic linearized schedule (when there
are multiple tasks running, only a single operation from one of the tasks is selected and allowed to
execute according to currently selected schedule, then after it completes, next operation is
selected, etc).

```rust
async fn handle_http_request() {
    parcheck::task!("name_of_this_request", {
        async {
            // ... code that isn't controlled by parcheck

            let value = parcheck::operation!("name_of_operation", {
                // ... perform an async operation: execute an SQL query, send a message, set/get
                // in redis, etc.
                async {}
            }).await;

            // ... some other code that isn't controlled

            parcheck::operation!("name_of_next_operation", {
                // ... another operation, that will be controlled by parcheck
                async {}
            }).await;
        }
    }).await;
}
```

We can test interleavings between two concurrent executions of this async function by adding a test case and running it via `parcheck::runner`.

```rust
#[tokio::test]
async fn test_concurrent_scenarios() {
    parcheck::runner()
        .run(["name_of_this_request", "name_of_this_request"], || async {
            join!(
                handle_http_request(),
                handle_http_request(),
            );

            // ... assert that state after 2 concurrent requests is as expected
        }).await;
}
```

Runner accepts names of tasks to test and starts to control their execution when they are started
(`parcheck::task` called). This test will run 2 concurrent `handle_http_request` multiple times,
each time sequence of operations will be different. If code panics under one of schedules, then
`parcheck` will print that schedule and it can be used to reproduce it again.
