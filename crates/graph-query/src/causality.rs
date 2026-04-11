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

// ─── Relationship causality ───────────────────────────────────────────────────

/// Walk backwards from the creation of `rel` to find all root changes
/// (stimuli — changes with no predecessors) that ultimately caused the
/// relationship to form.
///
/// Returns an empty `Vec` when:
/// - The relationship is not in the world.
/// - The relationship was pre-created with no `created_by` change (e.g.
///   inserted before the engine ran, or via `StructuralProposal::CreateRelationship`).
///
/// Returns `vec![created_by]` when the creation change itself is a root
/// stimulus (no predecessors). Otherwise walks the DAG from `created_by`
/// and returns all leaf ancestors.
///
/// This is the primary API for answering **"why does this relationship exist?"**
pub fn root_stimuli_for_relationship(world: &World, rel: RelationshipId) -> Vec<ChangeId> {
    let Some(created_by) = world
        .relationships()
        .get(rel)
        .and_then(|r| r.lineage.created_by)
    else {
        return Vec::new();
    };

    // If the creation change is itself a root stimulus, return it directly.
    if world.log().get(created_by).is_some_and(|c| c.predecessors.is_empty()) {
        return vec![created_by];
    }

    root_stimuli(world, created_by)
}

// ─── Relationship volatility ──────────────────────────────────────────────────

/// Activity volatility of `rel` over `[from_batch, to_batch]`.
///
/// Computed as the **standard deviation** of the activity slot (slot 0) across
/// all explicit `ChangeSubject::Relationship` changes in the range. A value
/// close to 0.0 indicates stable, steady coupling; a high value indicates
/// burst-driven or oscillating coupling.
///
/// Returns `0.0` when fewer than two explicit relationship changes exist in
/// the range — including when the relationship was only touched by
/// auto-emergence (which does not produce `ChangeSubject::Relationship` log
/// entries).
pub fn relationship_volatility(
    world: &World,
    rel: RelationshipId,
    from_batch: BatchId,
    to_batch: BatchId,
) -> f32 {
    let changes = changes_to_relationship_in_range(world, rel, from_batch, to_batch);
    let n = changes.len();
    if n < 2 {
        return 0.0;
    }
    let nf = n as f32;
    let activity = |c: &&Change| c.after.as_slice().first().copied().unwrap_or(0.0);
    let mean = changes.iter().map(activity).sum::<f32>() / nf;
    let variance = changes.iter().map(|c| (activity(c) - mean).powi(2)).sum::<f32>() / nf;
    variance.sqrt()
}

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
pub fn changes_to_locus_in_range(
    world: &World,
    locus: LocusId,
    from_batch: BatchId,
    to_batch: BatchId,
) -> Vec<&Change> {
    world
        .changes_to_locus(locus)
        .filter(|c| c.batch.0 >= from_batch.0 && c.batch.0 <= to_batch.0)
        .collect()
}

/// All changes that affected `rel` within the batch range `[from_batch,
/// to_batch]`, newest first.
pub fn changes_to_relationship_in_range(
    world: &World,
    rel: RelationshipId,
    from_batch: BatchId,
    to_batch: BatchId,
) -> Vec<&Change> {
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

    // ── root_stimuli_for_relationship ────────────────────────────────────────

    fn world_with_relationship_created_by(
        created_by: Option<u64>,
        root_pred: Vec<u64>,
    ) -> (World, RelationshipId) {
        use graph_core::{
            Endpoints, InfluenceKindId, Locus, LocusKindId, Relationship,
            RelationshipKindId, RelationshipLineage, StateVector,
        };
        let mut w = World::new();
        let rk: RelationshipKindId = InfluenceKindId(1);
        w.insert_locus(graph_core::Locus::new(LocusId(0), LocusKindId(1), StateVector::zeros(1)));
        w.insert_locus(Locus::new(LocusId(1), LocusKindId(1), StateVector::zeros(1)));

        // Push a root change (no predecessors) at id 0.
        push_change(&mut w, 0, 0, vec![], 0);
        // Push a derived change with `root_pred` as predecessors.
        if let Some(cid) = created_by {
            push_change(&mut w, cid, 1, root_pred, 1);
        }

        let rel_id = w.relationships_mut().mint_id();
        w.relationships_mut().insert(Relationship {
            id: rel_id,
            kind: rk,
            endpoints: Endpoints::Directed { from: LocusId(0), to: LocusId(1) },
            state: StateVector::from_slice(&[1.0, 0.0]),
            lineage: RelationshipLineage {
                created_by: created_by.map(ChangeId),
                last_touched_by: None,
                change_count: 1,
                kinds_observed: vec![rk],
            },
            last_decayed_batch: 0,
        });
        (w, rel_id)
    }

    #[test]
    fn root_stimuli_for_relationship_returns_empty_when_no_created_by() {
        let (w, rel_id) = world_with_relationship_created_by(None, vec![]);
        assert!(root_stimuli_for_relationship(&w, rel_id).is_empty());
    }

    #[test]
    fn root_stimuli_for_relationship_returns_stimulus_when_created_by_is_root() {
        // Change 1 has no predecessors → it IS the root stimulus.
        let (w, rel_id) = world_with_relationship_created_by(Some(1), vec![]);
        let roots = root_stimuli_for_relationship(&w, rel_id);
        assert_eq!(roots, vec![ChangeId(1)]);
    }

    #[test]
    fn root_stimuli_for_relationship_traces_through_predecessors() {
        // Chain: c0 (root) → c1 → c2 (created_by for rel)
        let mut w = World::new();
        push_change(&mut w, 0, 0, vec![], 0);     // root
        push_change(&mut w, 1, 1, vec![0], 1);    // derived
        push_change(&mut w, 2, 2, vec![1], 2);    // created_by

        use graph_core::{
            Endpoints, InfluenceKindId, Locus, LocusKindId, Relationship,
            RelationshipKindId, RelationshipLineage, StateVector,
        };
        let rk: RelationshipKindId = InfluenceKindId(1);
        for i in 0..3 {
            w.insert_locus(Locus::new(LocusId(i), LocusKindId(1), StateVector::zeros(1)));
        }
        let rel_id = w.relationships_mut().mint_id();
        w.relationships_mut().insert(Relationship {
            id: rel_id,
            kind: rk,
            endpoints: Endpoints::Directed { from: LocusId(0), to: LocusId(1) },
            state: StateVector::from_slice(&[1.0, 0.0]),
            lineage: RelationshipLineage {
                created_by: Some(ChangeId(2)),
                last_touched_by: None,
                change_count: 1,
                kinds_observed: vec![rk],
            },
            last_decayed_batch: 0,
        });

        let roots = root_stimuli_for_relationship(&w, rel_id);
        assert_eq!(roots, vec![ChangeId(0)]);
    }

    // ── relationship_volatility ──────────────────────────────────────────────

    fn world_with_rel_changes(activity_values: &[f32]) -> (World, RelationshipId) {
        use graph_core::{
            Change, ChangeSubject, Endpoints, InfluenceKindId, Locus, LocusKindId,
            Relationship, RelationshipKindId, RelationshipLineage, StateVector,
        };
        let rk: RelationshipKindId = InfluenceKindId(1);
        let mut w = World::new();
        w.insert_locus(Locus::new(LocusId(0), LocusKindId(1), StateVector::zeros(1)));
        w.insert_locus(Locus::new(LocusId(1), LocusKindId(1), StateVector::zeros(1)));

        let rel_id = w.relationships_mut().mint_id();
        w.relationships_mut().insert(Relationship {
            id: rel_id,
            kind: rk,
            endpoints: Endpoints::Directed { from: LocusId(0), to: LocusId(1) },
            state: StateVector::from_slice(&[1.0, 0.0]),
            lineage: RelationshipLineage {
                created_by: None,
                last_touched_by: None,
                change_count: activity_values.len() as u64,
                kinds_observed: vec![rk],
            },
            last_decayed_batch: 0,
        });

        for (batch, &act) in activity_values.iter().enumerate() {
            let cid = w.mint_change_id();
            w.append_change(Change {
                id: cid,
                subject: ChangeSubject::Relationship(rel_id),
                kind: InfluenceKindId(1),
                predecessors: vec![],
                before: StateVector::from_slice(&[0.0, 0.0]),
                after: StateVector::from_slice(&[act, 0.0]),
                batch: BatchId(batch as u64),
                wall_time: None,
                metadata: None,
            });
        }
        (w, rel_id)
    }

    #[test]
    fn relationship_volatility_zero_for_fewer_than_two_changes() {
        let (w, rel_id) = world_with_rel_changes(&[0.5]);
        assert_eq!(relationship_volatility(&w, rel_id, BatchId(0), BatchId(10)), 0.0);
    }

    #[test]
    fn relationship_volatility_zero_for_constant_activity() {
        let (w, rel_id) = world_with_rel_changes(&[0.5, 0.5, 0.5]);
        let v = relationship_volatility(&w, rel_id, BatchId(0), BatchId(10));
        assert!(v.abs() < 1e-5, "constant activity should have ~0 volatility, got {v}");
    }

    #[test]
    fn relationship_volatility_nonzero_for_variable_activity() {
        let (w, rel_id) = world_with_rel_changes(&[0.1, 0.9, 0.1, 0.9]);
        let v = relationship_volatility(&w, rel_id, BatchId(0), BatchId(10));
        assert!(v > 0.3, "alternating activity should have high volatility, got {v}");
    }
}
