use std::{
    env,
    error::Error,
    fmt,
    future::Future,
    panic::{self, AssertUnwindSafe},
    str::FromStr,
    time::Duration,
};

use fastrand::Rng;
use futures_util::{future::BoxFuture, join, FutureExt};

use crate::enabled::{
    controller::Controller,
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
    before_iter: Option<BeforeIter>,
    after_iter: Option<AfterIter>,
}

enum IterationConfig {
    Replay { trace: Trace },
    Iterate { max_iterations: u64 },
}

pub type PanicHandler = Box<dyn FnOnce(&Trace)>;
pub type BeforeStep = Box<dyn FnMut() -> BoxFuture<'static, ()>>;
pub type AfterStep = Box<dyn FnMut() -> BoxFuture<'static, ()>>;
pub type BeforeIter = Box<dyn FnMut() -> BoxFuture<'static, ()>>;
pub type AfterIter = Box<dyn FnMut() -> BoxFuture<'static, ()>>;

impl Default for Runner {
    fn default() -> Self {
        Self {
            iteration_config: IterationConfig::Iterate {
                max_iterations: u64::MAX,
            },
            on_panic: None,
            before_step: None,
            after_step: None,
            before_iter: None,
            after_iter: None,
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

    pub fn replay(mut self, trace: Trace) -> Self {
        self.iteration_config = IterationConfig::Replay { trace };
        self
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

    pub fn before_iter(mut self, before_iter: BeforeStep) -> Self {
        self.before_iter = Some(before_iter);
        self
    }

    pub fn after_iter(mut self, after_iter: AfterStep) -> Self {
        self.after_iter = Some(after_iter);
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
                    let mut steps_from_trace = trace.steps.into_iter();
                    let mut rng = Rng::new();

                    if let Some(before_iter) = &mut self.before_iter {
                        before_iter().await;
                    }

                    loop {
                        let _tasks = controller.ready(WAIT_TIMEOUT).await;
                        if let Some(before_step) = &mut self.before_step {
                            before_step().await;
                        }

                        let task_id = steps_from_trace
                            .next()
                            .map(|(task_id, _, _)| {
                                // TODO: check task name, op name
                                task_id
                            })
                            .or_else(|| {
                                let candidates = controller
                                    .tasks()
                                    .iter()
                                    .filter_map(|(task, state)| {
                                        state.can_execute().then_some(task.id())
                                    })
                                    .collect::<Vec<TaskId>>();

                                if candidates.is_empty() {
                                    return None;
                                }

                                Some(candidates[rng.usize(..candidates.len())])
                            });

                        let Some(task_id) = task_id else {
                            break;
                        };

                        controller.step_forward(task_id).await;
                        if let Some(after_step) = &mut self.after_step {
                            after_step().await;
                        }
                    }

                    controller.assert_finished();
                    drop(controller);

                    if let Some(after_iter) = &mut self.after_iter {
                        after_iter().await;
                    }
                };

                // TODO: handle panics
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

                if let Some(before_iter) = &mut self.before_iter {
                    before_iter().await;
                }

                loop {
                    let tasks = controller.ready(WAIT_TIMEOUT).await;
                    let Some(task_id) = cursor.visit_and_pick(tasks, &mut rng) else {
                        break;
                    };

                    let (task_name, op_name) = controller
                        .tasks()
                        .iter()
                        .find_map(|(task, state)| {
                            if task.id() == task_id {
                                let op_metadata = state
                                    .executable_op()
                                    .expect("task with chosen task_id isn't executable");
                                Some((task.name().clone(), OperationName(op_metadata.name.into())))
                            } else {
                                None
                            }
                        })
                        .expect("can't find task name & op name for chosen task");

                    trace.steps.push((task_id, task_name, op_name));
                    if let Some(before_step) = &mut self.before_step {
                        before_step().await;
                    }
                    controller.step_forward(task_id).await;
                    if let Some(after_step) = &mut self.after_step {
                        after_step().await;
                    }
                }

                controller.assert_finished();
                drop(controller);

                if let Some(after_iter) = &mut self.after_iter {
                    after_iter().await;
                }
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
    steps: Vec<(TaskId, TaskName, OperationName)>,
}

struct OperationName(String);

impl Trace {
    fn new() -> Self {
        Self { steps: Vec::new() }
    }
}

impl fmt::Display for Trace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.steps.is_empty() {
            return Ok(());
        }

        let (task_id, task_name, op_name) = &self.steps[0];
        write!(f, "{}:{}.{}", task_id.0, task_name.0, op_name.0)?;
        for (task_id, task_name, op_name) in &self.steps[1..] {
            write!(f, ",{}:{}.{}", task_id.0, task_name.0, op_name.0)?;
        }
        Ok(())
    }
}

impl FromStr for Trace {
    type Err = ParseTraceError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let steps = s
            .split(',')
            .map(|step| {
                let (task_id, names) = step.split_once(':').ok_or(ParseTraceError)?;
                let (task_name, op_name) = names.split_once('.').ok_or(ParseTraceError)?;

                let task_id = TaskId(task_id.parse().map_err(|_| ParseTraceError)?);
                Ok((
                    task_id,
                    TaskName(task_name.into()),
                    OperationName(op_name.into()),
                ))
            })
            .collect::<Result<Vec<_>, ParseTraceError>>()?;

        Ok(Self { steps })
    }
}

pub struct ParseTraceError;

impl fmt::Debug for ParseTraceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid trace")
    }
}

impl fmt::Display for ParseTraceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid trace")
    }
}

impl Error for ParseTraceError {}
