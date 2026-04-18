use std::collections::VecDeque;

use graph_core::{Change, ChangeId};
use rustc_hash::FxHashSet;

use super::ChangeLog;

pub(super) fn causal_ancestors(log: &ChangeLog, start: ChangeId) -> Vec<&Change> {
    let Some(root) = log.get(start) else {
        return Vec::new();
    };
    let mut visited: FxHashSet<ChangeId> = FxHashSet::default();
    visited.insert(start);
    let mut queue: VecDeque<ChangeId> = root.predecessors.iter().copied().collect();
    let mut result = Vec::new();

    while let Some(id) = queue.pop_front() {
        if visited.insert(id)
            && let Some(change) = log.get(id)
        {
            result.push(change);
            queue.extend(change.predecessors.iter().copied());
        }
    }
    result
}

pub(super) fn is_ancestor_of(log: &ChangeLog, ancestor: ChangeId, descendant: ChangeId) -> bool {
    if ancestor.0 >= descendant.0 {
        return false;
    }
    let Some(descendant_change) = log.get(descendant) else {
        return false;
    };
    let mut stack: Vec<ChangeId> = descendant_change
        .predecessors
        .iter()
        .copied()
        .filter(|&predecessor| predecessor.0 >= ancestor.0)
        .collect();
    let mut visited: FxHashSet<ChangeId> = FxHashSet::default();

    while let Some(id) = stack.pop() {
        if id == ancestor {
            return true;
        }
        if visited.insert(id)
            && let Some(change) = log.get(id)
        {
            stack.extend(
                change
                    .predecessors
                    .iter()
                    .copied()
                    .filter(|&predecessor| predecessor.0 >= ancestor.0),
            );
        }
    }
    false
}
