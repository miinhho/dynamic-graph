use graph_core::{ChangeId, RelationshipId};
use graph_world::World;
use rustc_hash::{FxHashMap, FxHashSet};

use super::types::CoarseTrail;

pub fn common_ancestors(world: &World, a: ChangeId, b: ChangeId) -> Vec<ChangeId> {
    let ancestors_a: FxHashSet<ChangeId> = causal_ancestors(world, a).into_iter().collect();
    causal_ancestors(world, b)
        .into_iter()
        .filter(|id| ancestors_a.contains(id))
        .collect()
}

pub fn causal_depth(world: &World, change_id: ChangeId) -> usize {
    let mut memo: FxHashMap<ChangeId, usize> = FxHashMap::default();
    let mut stack: Vec<(ChangeId, bool)> = vec![(change_id, false)];

    while let Some((cid, processed)) = stack.pop() {
        if processed {
            let depth = world
                .log()
                .get(cid)
                .map(|change| {
                    if change.predecessors.is_empty() {
                        0
                    } else {
                        change
                            .predecessors
                            .iter()
                            .map(|&predecessor| memo.get(&predecessor).copied().unwrap_or(0) + 1)
                            .max()
                            .unwrap_or(0)
                    }
                })
                .unwrap_or(0);
            memo.insert(cid, depth);
        } else if !memo.contains_key(&cid) {
            stack.push((cid, true));
            if let Some(change) = world.log().get(cid) {
                for &predecessor in &change.predecessors {
                    if !memo.contains_key(&predecessor) {
                        stack.push((predecessor, false));
                    }
                }
            }
        }
    }

    memo.get(&change_id).copied().unwrap_or(0)
}

pub fn root_stimuli_for_relationship(world: &World, rel: RelationshipId) -> Vec<ChangeId> {
    let Some(created_by) = world
        .relationships()
        .get(rel)
        .and_then(|relationship| relationship.lineage.created_by)
    else {
        return Vec::new();
    };

    if world
        .log()
        .get(created_by)
        .is_some_and(|change| change.predecessors.is_empty())
    {
        return vec![created_by];
    }

    root_stimuli(world, created_by)
}

pub fn causal_coarse_trail(world: &World, target: ChangeId) -> CoarseTrail {
    let log = world.log();
    let first_retained = log
        .iter()
        .next()
        .map(|change| change.id.0)
        .unwrap_or(u64::MAX);

    let fine_changes = log.causal_ancestors(target);
    let fine: Vec<ChangeId> = fine_changes.iter().map(|change| change.id).collect();
    let mut trimmed_loci: FxHashMap<graph_core::LocusId, ()> = FxHashMap::default();

    let mut check_preds = |preds: &[ChangeId]| {
        for &pred in preds {
            if pred.0 < first_retained
                && let Some(locus_id) = log.trimmed_locus_for(pred)
            {
                trimmed_loci.insert(locus_id, ());
            }
        }
    };

    if let Some(root) = log.get(target) {
        check_preds(&root.predecessors);
    }
    for change in &fine_changes {
        check_preds(&change.predecessors);
    }

    let coarse = trimmed_loci
        .keys()
        .flat_map(|&locus_id| log.trim_summaries_for_locus(locus_id))
        .cloned()
        .collect();

    CoarseTrail { fine, coarse }
}

pub fn causal_ancestors(world: &World, target: ChangeId) -> Vec<ChangeId> {
    world
        .log()
        .causal_ancestors(target)
        .into_iter()
        .map(|change| change.id)
        .collect()
}

pub fn is_ancestor_of(world: &World, ancestor: ChangeId, descendant: ChangeId) -> bool {
    world.log().is_ancestor_of(ancestor, descendant)
}

pub fn causal_descendants(world: &World, target: ChangeId) -> Vec<ChangeId> {
    use std::collections::VecDeque;

    let mut forward: FxHashMap<ChangeId, Vec<ChangeId>> = FxHashMap::default();
    for change in world.log().iter() {
        for &predecessor in &change.predecessors {
            forward.entry(predecessor).or_default().push(change.id);
        }
    }

    let mut visited: FxHashSet<ChangeId> = FxHashSet::default();
    let mut queue: VecDeque<ChangeId> = VecDeque::new();
    if let Some(children) = forward.get(&target) {
        for &child in children {
            if visited.insert(child) {
                queue.push_back(child);
            }
        }
    }
    while let Some(cid) = queue.pop_front() {
        if let Some(children) = forward.get(&cid) {
            for &child in children {
                if visited.insert(child) {
                    queue.push_back(child);
                }
            }
        }
    }

    visited.into_iter().collect()
}

pub fn root_stimuli(world: &World, target: ChangeId) -> Vec<ChangeId> {
    causal_ancestors(world, target)
        .into_iter()
        .filter(|&change_id| {
            world
                .log()
                .get(change_id)
                .is_some_and(|change| change.predecessors.is_empty())
        })
        .collect()
}
