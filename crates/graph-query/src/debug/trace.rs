use std::collections::VecDeque;

use rustc_hash::FxHashSet;

use graph_core::{BatchId, ChangeId, LocusId};
use graph_world::World;

use super::{CausalStep, CausalTrace};

pub fn causal_trace(world: &World, locus: LocusId, batch: BatchId) -> CausalTrace {
    let Some(start_change) = find_trace_start(world, locus, batch) else {
        return CausalTrace {
            target: locus,
            batch,
            steps: Vec::new(),
            truncated: false,
        };
    };

    let mut state = TraceState::new(start_change.id);
    while let Some((change_id, depth)) = state.pop_front() {
        state.observe_change(world, change_id, depth);
    }

    CausalTrace {
        target: locus,
        batch,
        steps: state.steps,
        truncated: state.truncated,
    }
}

fn find_trace_start(world: &World, locus: LocusId, batch: BatchId) -> Option<&graph_core::Change> {
    world
        .changes_to_locus(locus)
        .find(|change| change.batch.0 <= batch.0)
}

struct TraceState {
    queue: VecDeque<(ChangeId, usize)>,
    visited: FxHashSet<ChangeId>,
    steps: Vec<CausalStep>,
    truncated: bool,
}

impl TraceState {
    fn new(start_id: ChangeId) -> Self {
        let mut queue = VecDeque::new();
        let mut visited = FxHashSet::default();
        queue.push_back((start_id, 0));
        visited.insert(start_id);

        Self {
            queue,
            visited,
            steps: Vec::new(),
            truncated: false,
        }
    }

    fn pop_front(&mut self) -> Option<(ChangeId, usize)> {
        self.queue.pop_front()
    }

    fn observe_change(&mut self, world: &World, change_id: ChangeId, depth: usize) {
        let Some(change) = world.log().get(change_id) else {
            self.truncated = true;
            return;
        };

        self.flag_trimmed_predecessors(world, &change.predecessors);
        self.steps.push(CausalStep {
            depth,
            change_id: change.id,
            subject: change.subject.clone(),
            kind: change.kind,
            before: change.before.clone(),
            after: change.after.clone(),
            batch: change.batch,
            predecessor_ids: change.predecessors.clone(),
        });
        self.enqueue_predecessors(&change.predecessors, depth + 1);
    }

    fn flag_trimmed_predecessors(&mut self, world: &World, predecessors: &[ChangeId]) {
        if predecessors
            .iter()
            .any(|&predecessor| world.log().get(predecessor).is_none())
        {
            self.truncated = true;
        }
    }

    fn enqueue_predecessors(&mut self, predecessors: &[ChangeId], depth: usize) {
        for &predecessor in predecessors {
            if self.visited.insert(predecessor) {
                self.queue.push_back((predecessor, depth));
            }
        }
    }
}
