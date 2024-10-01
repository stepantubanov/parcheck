use fastrand::Rng;
use std::ops::Range;

use crate::enabled::{
    controller::TaskState,
    task::{Task, TaskId, TaskName},
};

pub(crate) struct ScheduleTree {
    nodes: Vec<Node>,
    roots: usize,
    unvisited_leafs: Vec<Path>,
}

#[derive(Debug, Copy, Clone)]
pub(crate) struct NodeId(usize);

struct Node {
    // TODO: spawn tasks
    #[allow(dead_code)]
    task_name: TaskName,
    state: NodeState,
}

enum NodeState {
    Unvisited,
    Visited { children: Range<usize> },
    Unreachable { reason: &'static str },
}

struct Path(Vec<TaskId>);

impl ScheduleTree {
    pub(crate) fn new(roots: &[TaskName]) -> Self {
        let nodes = roots
            .iter()
            .map(|task_name| Node {
                task_name: task_name.clone(),
                state: NodeState::Unvisited,
            })
            .collect::<Vec<Node>>();

        Self {
            roots: nodes.len(),
            unvisited_leafs: (0..nodes.len())
                .map(|idx| Path(vec![TaskId(idx)]))
                .collect(),
            nodes,
        }
    }

    pub(crate) fn has_unfinished_paths(&self) -> bool {
        !self.unvisited_leafs.is_empty()
    }

    pub(crate) fn pick_unfinished_path(&mut self, rng: &mut Rng) -> Option<PathCursor<'_>> {
        if self.unvisited_leafs.is_empty() {
            return None;
        }

        let path = rng.usize(..self.unvisited_leafs.len());

        Some(PathCursor {
            tree: self,
            state: CursorState::Path {
                at: None,
                path,
                depth: 0,
            },
        })
    }

    fn add_nodes(&mut self, nodes: impl IntoIterator<Item = Node>) -> Range<usize> {
        let start = self.nodes.len();
        self.nodes.extend(nodes);
        start..self.nodes.len()
    }
}

pub(crate) struct PathCursor<'a> {
    tree: &'a mut ScheduleTree,
    state: CursorState,
}

enum CursorState {
    Path {
        at: Option<NodeId>,
        path: usize,
        depth: usize,
    },
    Finished,
}

impl<'a> PathCursor<'a> {
    pub(crate) fn visit_and_pick(
        &mut self,
        tasks: &[(Task, TaskState)],
        rng: &mut Rng,
    ) -> Option<TaskId> {
        let CursorState::Path { at, path, depth } = &mut self.state else {
            panic!("visit() called in wrong state");
        };

        if let Some(node_id) = at {
            match &self.tree.nodes[node_id.0].state {
                NodeState::Visited { .. } => {
                    // TODO: compare `tasks` and children
                }
                NodeState::Unreachable { reason } => {
                    panic!("visited node marked as unreachable ({reason})");
                }
                NodeState::Unvisited => {
                    let children = self.tree.add_nodes(tasks_to_nodes(tasks));
                    self.tree.nodes[node_id.0].state = NodeState::Visited {
                        children: children.clone(),
                    };

                    assert_eq!(*depth, self.tree.unvisited_leafs[*path].0.len());

                    let unvisited = tasks
                        .iter()
                        .filter_map(|(task, state)| state.can_execute().then_some(task.id()));

                    let num_unvisited = unvisited.clone().count();
                    if num_unvisited == 0 {
                        self.tree.unvisited_leafs.swap_remove(*path);
                        self.state = CursorState::Finished;
                        return None;
                    }

                    let next_task_id = unvisited.clone().nth(rng.usize(..num_unvisited)).unwrap();
                    for child_task_id in unvisited.filter(|id| *id != next_task_id) {
                        let mut path = Path(self.tree.unvisited_leafs[*path].0.clone());
                        path.0.push(child_task_id);

                        self.tree.unvisited_leafs.push(path);
                    }

                    self.tree.unvisited_leafs[*path].0.push(next_task_id);
                }
            }
        } else {
            assert_eq!(tasks.len(), self.tree.roots);

            for (i, (_, task_state)) in tasks.iter().enumerate() {
                let state = &mut self.tree.nodes[i].state;
                if matches!(state, NodeState::Unvisited) {
                    *state = task_state_to_node_state(task_state);
                }
            }

            let root_idx = self.tree.unvisited_leafs[*path].0[0];
            if matches!(
                self.tree.nodes[root_idx.0].state,
                NodeState::Unreachable { .. }
            ) {
                self.tree.unvisited_leafs.swap_remove(*path);
                self.state = CursorState::Finished;
                return None;
            }
        };

        let path = &self.tree.unvisited_leafs[*path];
        if *depth < path.0.len() {
            let task_id = path.0[*depth];
            *depth += 1;
            match at {
                Some(node_id) => {
                    let NodeState::Visited { children } = &self.tree.nodes[node_id.0].state else {
                        panic!("created path through unvisited nodes");
                    };
                    *node_id = NodeId(children.start + task_id.0);
                }
                None => *at = Some(NodeId(task_id.0)),
            }
            Some(task_id)
        } else {
            None
        }
    }
}

fn tasks_to_nodes(tasks: &[(Task, TaskState)]) -> impl Iterator<Item = Node> + '_ {
    tasks.iter().map(|(task, state)| Node {
        task_name: task.name().clone(),
        state: task_state_to_node_state(state),
    })
}

fn task_state_to_node_state(task_state: &TaskState) -> NodeState {
    match task_state {
        TaskState::NotStarted
        | TaskState::ExecutingOutsideOperation
        | TaskState::ExecutingOperation { .. }
        | TaskState::Invalid => unreachable!(),
        TaskState::WaitingToStartOperation { blocked_locks, .. } if blocked_locks.is_empty() => {
            NodeState::Unvisited
        }
        TaskState::WaitingToStartOperation { .. } => NodeState::Unreachable {
            reason: "blocked by locks",
        },
        TaskState::Finished => NodeState::Unreachable {
            reason: "task finished",
        },
    }
}
