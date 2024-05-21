use std::{env, future::Future};

use fastrand::Rng;

use crate::{controller::Controller, schedule_tree::ScheduleTree, task::TaskName};

pub fn runner() -> Runner {
    Runner::from_env()
}

pub struct Runner {
    max_iterations: u64,
}

impl Default for Runner {
    fn default() -> Self {
        Self {
            max_iterations: u64::MAX,
        }
    }
}

impl Runner {
    pub fn from_env() -> Self {
        let mut runner = Runner::default();
        if let Ok(max_iterations) = env::var("PARCHECK_MAX_ITERATIONS") {
            runner.max_iterations = max_iterations
                .parse()
                .expect("failed to parse PARCHECK_MAX_ITERATIONS");
        }
        runner
    }

    pub fn max_iterations(self, max_iterations: u64) -> Self {
        Self {
            max_iterations,
            ..self
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

        let mut schedule_tree = ScheduleTree::new(&initial_tasks);

        while schedule_tree.has_unfinished_paths() {
            let mut controller = Controller::register(&initial_tasks);

            let task = tokio::spawn(async move {
                let mut rng = Rng::new();
                let mut cursor = schedule_tree.pick_unfinished_path(&mut rng).unwrap();

                loop {
                    let tasks = controller.ready().await;
                    let Some(task_id) = cursor.visit_and_pick(tasks, &mut rng) else {
                        break;
                    };

                    controller.step_forward(task_id).await;
                }

                schedule_tree
            });

            state = f(state).await;
            schedule_tree = task.await.unwrap();
        }
        state
    }
}
