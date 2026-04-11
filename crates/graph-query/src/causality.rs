//! Causal DAG queries over the change log.
//!
//! The `ChangeLog` records causal predecessor edges for every committed
//! change. These functions let callers walk that graph: find all ancestors
//! of a change, or the complete set of changes that affected a locus within
//! a batch range.
//!
//! All queries are read-only over `&World`.

use graph_core::{BatchId, Change, ChangeId, LocusId, RelationshipId};
use graph_world::World;

// ─── Ancestor queries ─────────────────────────────────────────────────────────

/// All causal ancestor `ChangeId`s of `target`, via BFS in the predecessor
/// graph. The result is deduplicated but unordered. Does not include `target`
/// itself.
///
/// Stops at changes that have been trimmed from the log (trimmed ranges are
/// represented as tombstones in the log; the BFS halts when it encounters one).
///
/// Complexity: O(ancestors).
pub fn causal_ancestors(world: &World, target: ChangeId) -> Vec<ChangeId> {
    world.log().causal_ancestors(target).into_iter().map(|c| c.id).collect()
}

/// True iff `ancestor` is a causal ancestor of `descendant` in the change
/// log. Equivalent to `causal_ancestors(world, descendant).contains(&ancestor)`
/// but short-circuits on the first confirming path found.
///
/// Returns `false` if either ID is not in the log.
pub fn is_ancestor_of(world: &World, ancestor: ChangeId, descendant: ChangeId) -> bool {
    world.log().is_ancestor_of(ancestor, descendant)
}

// ─── Locus change range ───────────────────────────────────────────────────────

/// All changes that affected `locus` within the (inclusive) batch range
/// `[from_batch, to_batch]`, newest first.
///
/// Wraps `ChangeLog::changes_to_locus` with a batch-range filter. Useful for
/// auditing what happened to a specific locus over a time window.
pub fn changes_to_locus_in_range<'w>(
    world: &'w World,
    locus: LocusId,
    from_batch: BatchId,
    to_batch: BatchId,
) -> Vec<&'w Change> {
    world
        .changes_to_locus(locus)
        .filter(|c| c.batch.0 >= from_batch.0 && c.batch.0 <= to_batch.0)
        .collect()
}

/// All changes that affected `rel` within the batch range `[from_batch,
/// to_batch]`, newest first.
pub fn changes_to_relationship_in_range<'w>(
    world: &'w World,
    rel: RelationshipId,
    from_batch: BatchId,
    to_batch: BatchId,
) -> Vec<&'w Change> {
    world
        .changes_to_relationship(rel)
        .filter(|c| c.batch.0 >= from_batch.0 && c.batch.0 <= to_batch.0)
        .collect()
}

// ─── Source trace ─────────────────────────────────────────────────────────────

/// Walk backwards from `target` to find all root changes (stimuli — changes
/// with no predecessors) that are ancestors of `target`.
///
/// These are the external inputs that ultimately caused `target` to fire.
/// Returns an empty `Vec` when `target` itself is a stimulus.
pub fn root_stimuli(world: &World, target: ChangeId) -> Vec<ChangeId> {
    let ancestors = causal_ancestors(world, target);
    ancestors
        .into_iter()
        .filter(|&cid| {
            world
                .log()
                .get(cid)
                .is_some_and(|c| c.predecessors.is_empty())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{
        BatchId, Change, ChangeId, ChangeSubject, InfluenceKindId, LocusId, StateVector,
    };
    use graph_world::World;

    fn push_change(world: &mut World, id: u64, locus: u64, preds: Vec<u64>, batch: u64) -> ChangeId {
        let cid = ChangeId(id);
        world.log_mut().append(Change {
            id: cid,
            subject: ChangeSubject::Locus(LocusId(locus)),
            kind: InfluenceKindId(1),
            predecessors: preds.into_iter().map(ChangeId).collect(),
            before: StateVector::zeros(1),
            after: StateVector::from_slice(&[0.5]),
            batch: BatchId(batch),
            wall_time: None,
            metadata: None,
        });
        cid
    }

    // Causal chain: c0 (root) → c1 → c2
    fn chain_world() -> World {
        let mut w = World::new();
        push_change(&mut w, 0, 0, vec![], 0);
        push_change(&mut w, 1, 1, vec![0], 1);
        push_change(&mut w, 2, 2, vec![1], 2);
        w
    }

    #[test]
    fn causal_ancestors_returns_all_predecessors() {
        let w = chain_world();
        let mut ancestors = causal_ancestors(&w, ChangeId(2));
        ancestors.sort();
        assert_eq!(ancestors, vec![ChangeId(0), ChangeId(1)]);
    }

    #[test]
    fn causal_ancestors_of_root_is_empty() {
        let w = chain_world();
        assert!(causal_ancestors(&w, ChangeId(0)).is_empty());
    }

    #[test]
    fn is_ancestor_of_detects_transitivity() {
        let w = chain_world();
        assert!(is_ancestor_of(&w, ChangeId(0), ChangeId(2)));
        assert!(is_ancestor_of(&w, ChangeId(1), ChangeId(2)));
        assert!(!is_ancestor_of(&w, ChangeId(2), ChangeId(0)));
    }

    #[test]
    fn changes_to_locus_in_range_filters_by_batch() {
        let mut w = World::new();
        // Three changes to locus 0, at batches 1, 3, 5.
        push_change(&mut w, 0, 0, vec![], 1);
        push_change(&mut w, 1, 0, vec![], 3);
        push_change(&mut w, 2, 0, vec![], 5);

        let range = changes_to_locus_in_range(&w, LocusId(0), BatchId(2), BatchId(4));
        assert_eq!(range.len(), 1);
        assert_eq!(range[0].batch, BatchId(3));
    }

    #[test]
    fn root_stimuli_finds_origin_changes() {
        let w = chain_world();
        let roots = root_stimuli(&w, ChangeId(2));
        assert_eq!(roots, vec![ChangeId(0)]);
    }

    #[test]
    fn root_stimuli_empty_for_stimulus_itself() {
        let w = chain_world();
        assert!(root_stimuli(&w, ChangeId(0)).is_empty());
    }
}
