use core::fmt;
use std::{
    env,
    future::Future,
    num::ParseIntError,
    panic::{self, AssertUnwindSafe},
    str::FromStr,
    time::Duration,
};

use fastrand::Rng;
use futures_util::{future::BoxFuture, join, FutureExt};

use crate::enabled::{
    controller::{Controller, TaskState},
    schedule_tree::ScheduleTree,
    task::{TaskId, TaskName},
};

pub fn runner() -> Runner {
    Runner::from_env()
}

pub struct Runner {
    iteration_config: IterationConfig,
    on_panic: Option<PanicHandler>,
    before_step: Option<BeforeStep>,
    after_step: Option<AfterStep>,
}

enum IterationConfig {
    Replay { trace: Trace },
    Iterate { max_iterations: u64 },
}

pub type PanicHandler = Box<dyn FnOnce(&Trace)>;
pub type BeforeStep = Box<dyn FnMut() -> BoxFuture<'static, ()>>;
pub type AfterStep = Box<dyn FnMut() -> BoxFuture<'static, ()>>;

impl Default for Runner {
    fn default() -> Self {
        Self {
            iteration_config: IterationConfig::Iterate {
                max_iterations: u64::MAX,
            },
            on_panic: None,
            before_step: None,
            after_step: None,
        }
    }
}

impl Runner {
    pub fn from_env() -> Self {
        let mut runner = Self::default();

        if let Ok(trace) = env::var("PARCHECK_REPLAY") {
            let trace = trace.parse().expect("can't parse PARCHECK_REPLAY");
            runner.iteration_config = IterationConfig::Replay { trace };
        } else if let Ok(max_iterations) = env::var("PARCHECK_MAX_ITERATIONS") {
            runner.iteration_config = IterationConfig::Iterate {
                max_iterations: max_iterations
                    .parse()
                    .expect("failed to parse PARCHECK_MAX_ITERATIONS"),
            };
        }

        runner
    }

    pub fn max_iterations(mut self, max_iterations: u64) -> Self {
        self.iteration_config = IterationConfig::Iterate { max_iterations };
        self
    }

    pub fn on_panic(mut self, on_panic: PanicHandler) -> Self {
        self.on_panic = Some(on_panic);
        self
    }

    pub fn before_step(mut self, before_step: BeforeStep) -> Self {
        self.before_step = Some(before_step);
        self
    }

    pub fn after_step(mut self, after_step: AfterStep) -> Self {
        self.after_step = Some(after_step);
        self
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
        mut self,
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
        // TODO: add to config
        const WAIT_TIMEOUT: Duration = Duration::from_secs(5);

        let initial_tasks: Vec<TaskName> = initial_tasks
            .into_iter()
            .map(|name| TaskName(name.into()))
            .collect();

        let max_iterations = match self.iteration_config {
            IterationConfig::Replay { trace } => {
                let mut controller = Controller::register(&initial_tasks);

                let control = async {
                    for &task_id in &trace.task_ids {
                        let _tasks = controller.ready(WAIT_TIMEOUT).await;
                        if let Some(before_step) = &mut self.before_step {
                            before_step().await;
                        }
                        controller.step_forward(task_id).await;
                        if let Some(after_step) = &mut self.after_step {
                            after_step().await;
                        }
                    }
                };

                (state, _) = join!(f(state), control);
                return state;
            }
            IterationConfig::Iterate {
                max_iterations: max_iter,
            } => max_iter,
        };

        let mut schedule_tree = ScheduleTree::new(&initial_tasks);
        let mut iter = 0;

        while schedule_tree.has_unfinished_paths() && iter < max_iterations {
            let mut controller = Controller::register(&initial_tasks);
            let mut trace = Trace::new();

            let control = async {
                let mut rng = Rng::new();
                let mut cursor = schedule_tree.pick_unfinished_path(&mut rng).unwrap();

                loop {
                    let tasks = controller.ready(WAIT_TIMEOUT).await;
                    let Some(task_id) = cursor.visit_and_pick(tasks, &mut rng) else {
                        break;
                    };

                    trace.task_ids.push(task_id);
                    if let Some(before_step) = &mut self.before_step {
                        before_step().await;
                    }
                    controller.step_forward(task_id).await;
                    if let Some(after_step) = &mut self.after_step {
                        after_step().await;
                    }
                }

                // TODO: move this logic
                if !controller
                    .tasks()
                    .iter()
                    .all(|(_, state)| matches!(state, TaskState::Finished))
                {
                    let in_progress_tasks = controller
                        .tasks()
                        .iter()
                        .filter_map(|(task, state)| match state {
                            TaskState::InsideOperation { metadata } => Some(format!(
                                "task '{}' in operation '{}'",
                                task.name().0,
                                metadata.name
                            )),
                            _ => None,
                        })
                        .collect::<Vec<_>>();
                    let blocked_tasks = controller
                        .tasks()
                        .iter()
                        .filter_map(|(task, state)| match state {
                            TaskState::WaitingForPermit {
                                metadata,
                                blocked_locks,
                                ..
                            } if !blocked_locks.is_empty() => Some(format!(
                                "task '{}' in operation '{}' - blocked by locks",
                                task.name().0,
                                metadata.name
                            )),
                            _ => None,
                        })
                        .collect::<Vec<_>>();

                    panic!("some tasks did not finish. in progress tasks: {in_progress_tasks:?}, blocked tasks: {blocked_tasks:?}");
                }

                drop(controller);
            };

            let result = AssertUnwindSafe(async {
                (state, _) = join!(f(state), control);
                state
            })
            .catch_unwind()
            .await;

            state = match result {
                Ok(v) => v,
                Err(error) => {
                    if let Some(on_panic) = self.on_panic {
                        on_panic(&trace);
                    } else {
                        eprintln!(
                            "note: use `PARCHECK_REPLAY=\"{trace}\"` to replay the same schedule"
                        );
                    }
                    panic::resume_unwind(error);
                }
            };

            iter += 1;
        }
        state
    }
}

pub struct Trace {
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
