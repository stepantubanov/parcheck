use core::fmt;
use std::{
    env,
    future::Future,
    num::ParseIntError,
    panic::{self, AssertUnwindSafe},
    str::FromStr,
};

use fastrand::Rng;
use futures_util::join;
use futures_util::FutureExt;

use crate::enabled::{
    controller::Controller,
    schedule_tree::ScheduleTree,
    task::{TaskId, TaskName},
};

pub fn runner() -> Runner {
    Runner::from_env()
}

pub struct Runner {
    config: Config,
}

enum Config {
    Iterate(IterateConfig),
    Replay { trace: Trace },
}

struct IterateConfig {
    max_iterations: u64,
}

impl Runner {
    pub fn from_env() -> Self {
        if let Ok(trace) = env::var("PARCHECK_REPLAY") {
            let trace = trace.parse().expect("can't parse PARCHECK_REPLAY");
            return Self {
                config: Config::Replay { trace },
            };
        }

        let mut config = IterateConfig {
            max_iterations: u64::MAX,
        };
        if let Ok(max_iterations) = env::var("PARCHECK_MAX_ITERATIONS") {
            config.max_iterations = max_iterations
                .parse()
                .expect("failed to parse PARCHECK_MAX_ITERATIONS");
        }
        Self {
            config: Config::Iterate(config),
        }
    }

    pub fn max_iterations(self, max_iterations: u64) -> Self {
        match self.config {
            Config::Iterate(_) => Self {
                config: Config::Iterate(IterateConfig { max_iterations }),
            },
            Config::Replay { .. } => self,
        }
    }

    pub async fn run<'a, I, F, Fut>(self, initial_tasks: I, mut f: F)
    where
        I: IntoIterator,
        I::Item: Into<String>,
        F: FnMut() -> Fut,
        Fut: Future<Output = ()>,
    {
        self.run_with_state(initial_tasks, (), |()| f()).await
    }

    pub async fn run_with_state<'a, T, I, F, Fut>(
        self,
        initial_tasks: I,
        mut state: T,
        mut f: F,
    ) -> T
    where
        I: IntoIterator,
        I::Item: Into<String>,
        F: FnMut(T) -> Fut,
        Fut: Future<Output = T>,
    {
        let initial_tasks: Vec<TaskName> = initial_tasks
            .into_iter()
            .map(|name| TaskName(name.into()))
            .collect();

        let config = match self.config {
            Config::Replay { trace } => {
                let mut controller = Controller::register(&initial_tasks);

                let control = async {
                    for &task_id in &trace.task_ids {
                        let _tasks = controller.ready().await;
                        controller.step_forward(task_id).await;
                    }
                };

                let result = AssertUnwindSafe(async {
                    (state, _) = join!(f(state), control);
                    state
                })
                .catch_unwind()
                .await;

                state = unwrap_or_print_trace(&trace, result);
                return state;
            }
            Config::Iterate(config) => config,
        };

        let mut schedule_tree = ScheduleTree::new(&initial_tasks);
        let mut iter = 0;

        while schedule_tree.has_unfinished_paths() && iter < config.max_iterations {
            let mut controller = Controller::register(&initial_tasks);
            let mut trace = Trace::new();

            let control = async {
                let mut rng = Rng::new();
                let mut cursor = schedule_tree.pick_unfinished_path(&mut rng).unwrap();

                loop {
                    let tasks = controller.ready().await;
                    let Some(task_id) = cursor.visit_and_pick(tasks, &mut rng) else {
                        break;
                    };

                    trace.task_ids.push(task_id);
                    controller.step_forward(task_id).await;
                }
            };

            let result = AssertUnwindSafe(async {
                (state, _) = join!(f(state), control);
                state
            })
            .catch_unwind()
            .await;

            state = unwrap_or_print_trace(&trace, result);
            iter += 1;
        }
        state
    }
}

struct Trace {
    task_ids: Vec<TaskId>,
}

impl Trace {
    fn new() -> Self {
        Self {
            task_ids: Vec::new(),
        }
    }
}

impl fmt::Display for Trace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.task_ids.is_empty() {
            return Ok(());
        }

        write!(f, "{}", self.task_ids[0].0)?;
        for task_id in &self.task_ids[1..] {
            write!(f, ",{}", task_id.0)?;
        }
        Ok(())
    }
}

impl FromStr for Trace {
    type Err = ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self {
            task_ids: s
                .split(',')
                .map(|part| part.parse().map(TaskId))
                .collect::<Result<Vec<_>, _>>()?,
        })
    }
}

fn unwrap_or_print_trace<T>(trace: &Trace, result: std::thread::Result<T>) -> T {
    match result {
        Ok(v) => v,
        Err(error) => {
            eprintln!("Use PARCHECK_REPLAY=\"{trace}\" to replay the same schedule");
            panic::resume_unwind(error);
        }
    }
}
