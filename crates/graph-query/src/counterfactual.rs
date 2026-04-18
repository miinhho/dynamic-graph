//! Counterfactual relationship queries (D3).
//!
//! Two levels of counterfactual analysis:
//!
//! **Structural queries** (`relationships_caused_by`, `relationships_absent_without`):
//! Lightweight forward-causal analysis — identify which relationships have
//! causal paths back to a set of root stimuli.
//!
//! **Structural replay** (`counterfactual_replay`): Given a set of `ChangeId`s
//! to remove, compute the full structural impact:
//! - All suppressed changes (roots + their descendants)
//! - Relationships that would be absent
//! - The divergence batch (earliest suppressed batch)
//!
//! This is a pure read operation over the existing ChangeLog and relationship
//! lineage — no engine re-simulation is performed. The result is a
//! [`CounterfactualDiff`] rather than a `WorldDiff`; re-simulation from the
//! divergence point is left to callers who have access to the engine.
//!
//! ## Example
//!
//! ```ignore
//! let diff = graph_query::counterfactual_replay(&world, vec![stimulus_change_id]);
//! println!(
//!     "{} changes suppressed, {} relationships absent, divergence at {:?}",
//!     diff.suppressed_changes.len(),
//!     diff.absent_relationships.len(),
//!     diff.divergence_batch,
//! );
//! ```
//!
//! ## Original structural queries
//!
//! Answers the question: *"which relationships would not exist if stimulus X
//! had never happened?"*
//!
//! The engine uses predecessor links to record causality: every auto-emerged
//! relationship touch includes the `ChangeId`s that triggered it. By walking
//! **forward** from a set of root stimuli (using [`causal_descendants`]) we
//! can find all changes — and thus all relationship creations — that descend
//! from those stimuli.
//!
//! ## Limitation: multi-causal paths
//!
//! A relationship's creation change may have multiple predecessor paths. If
//! relationship R was caused by both stimulus A and stimulus B, removing A
//! alone would not eliminate R — B would still create it. This module does
//! **not** model multi-causal paths. The functions here return the
//! *maximally pessimistic* set: relationships that have *at least one*
//! causal path back to the given stimuli. Callers interested in
//! *exclusively* caused relationships must intersect the result against
//! relationships whose creation changes have *no* other causal predecessors.
//!
//! ## Example
//!
//! ```ignore
//! // Stimulate two sensory neurons and observe which relationships were caused.
//! let root_changes = world.log().batch(batch_id).iter().map(|c| c.id).collect::<Vec<_>>();
//! let caused = graph_query::relationships_caused_by(&world, &root_changes);
//! let absent_without = graph_query::relationships_absent_without(&world, &root_changes);
//! println!("{} relationships would vanish", absent_without.len());
//! ```

mod adapter;
mod analysis;
mod builder;
mod diff;

pub use analysis::{counterfactual_replay, relationships_absent_without, relationships_caused_by};
pub use builder::{CounterfactualQuery, counterfactual};
pub use diff::CounterfactualDiff;

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{
        BatchId, Change, ChangeId, ChangeSubject, Endpoints, InfluenceKindId, Locus, LocusId,
        LocusKindId, Relationship, RelationshipId, RelationshipLineage, StateVector,
    };
    use graph_world::World;

    fn make_world_with_linear_causal_chain() -> (World, RelationshipId, ChangeId) {
        // Creates a minimal world where:
        //   change 0 (locus) → change 1 (relationship created_by=0) → change 2 (locus)
        // The relationship is causally downstream of change 0.
        let mut world = World::new();
        world.insert_locus(Locus::new(
            LocusId(0),
            LocusKindId(1),
            StateVector::from_slice(&[1.0]),
        ));
        world.insert_locus(Locus::new(
            LocusId(1),
            LocusKindId(1),
            StateVector::from_slice(&[0.0]),
        ));

        let rel_id = world.relationships_mut().mint_id();
        world.relationships_mut().insert(Relationship {
            id: rel_id,
            kind: InfluenceKindId(1),
            endpoints: Endpoints::Directed {
                from: LocusId(0),
                to: LocusId(1),
            },
            state: StateVector::from_slice(&[0.5, 0.0]),
            lineage: RelationshipLineage {
                created_by: Some(ChangeId(1)),
                last_touched_by: Some(ChangeId(1)),
                change_count: 1,
                kinds_observed: smallvec::SmallVec::new(),
            },
            created_batch: BatchId(1),
            last_decayed_batch: 0,
            metadata: None,
        });

        // Commit three changes: root (0), relationship touch (1), downstream locus (2).
        let root_change = Change {
            id: ChangeId(0),
            subject: ChangeSubject::Locus(LocusId(0)),
            kind: InfluenceKindId(0),
            predecessors: vec![],
            before: StateVector::from_slice(&[0.0]),
            after: StateVector::from_slice(&[1.0]),
            batch: BatchId(1),
            wall_time: None,
            metadata: None,
        };
        let rel_change = Change {
            id: ChangeId(1),
            subject: ChangeSubject::Relationship(rel_id),
            kind: InfluenceKindId(1),
            predecessors: vec![ChangeId(0)],
            before: StateVector::from_slice(&[0.0, 0.0]),
            after: StateVector::from_slice(&[0.5, 0.0]),
            batch: BatchId(1),
            wall_time: None,
            metadata: None,
        };
        let downstream_change = Change {
            id: ChangeId(2),
            subject: ChangeSubject::Locus(LocusId(1)),
            kind: InfluenceKindId(1),
            predecessors: vec![ChangeId(1)],
            before: StateVector::from_slice(&[0.0]),
            after: StateVector::from_slice(&[0.5]),
            batch: BatchId(1),
            wall_time: None,
            metadata: None,
        };

        world.log_mut().append(root_change);
        world.log_mut().append(rel_change);
        world.log_mut().append(downstream_change);

        (world, rel_id, ChangeId(0))
    }

    #[test]
    fn relationships_caused_by_finds_downstream_relationship() {
        let (world, rel_id, root) = make_world_with_linear_causal_chain();
        let caused = relationships_caused_by(&world, &[root]);
        assert!(
            caused.contains(&rel_id),
            "relationship should be in caused set"
        );
    }

    #[test]
    fn relationships_absent_without_finds_created_relationship() {
        let (world, rel_id, root) = make_world_with_linear_causal_chain();
        let absent = relationships_absent_without(&world, &[root]);
        assert!(
            absent.contains(&rel_id),
            "relationship should be absent without root"
        );
    }

    #[test]
    fn empty_roots_returns_empty() {
        let (world, _, _) = make_world_with_linear_causal_chain();
        let caused = relationships_caused_by(&world, &[]);
        assert!(caused.is_empty());
        let absent = relationships_absent_without(&world, &[]);
        assert!(absent.is_empty());
    }

    #[test]
    fn unrelated_root_does_not_cause_relationship() {
        let (world, rel_id, _) = make_world_with_linear_causal_chain();
        // ChangeId(2) is downstream of the relationship, not its cause.
        let caused = relationships_caused_by(&world, &[ChangeId(2)]);
        assert!(!caused.contains(&rel_id));
    }

    // ── D3: counterfactual_replay ─────────────────────────────────────────────

    #[test]
    fn replay_empty_roots_returns_empty_diff() {
        let (world, _, _) = make_world_with_linear_causal_chain();
        let diff = counterfactual_replay(&world, vec![]);
        assert!(diff.is_empty());
        assert!(diff.absent_relationships.is_empty());
        assert!(diff.divergence_batch.is_none());
    }

    #[test]
    fn replay_finds_absent_relationship() {
        let (world, rel_id, root) = make_world_with_linear_causal_chain();
        let diff = counterfactual_replay(&world, vec![root]);
        assert!(
            diff.absent_relationships.contains(&rel_id),
            "relationship should be absent in counterfactual world"
        );
    }

    #[test]
    fn replay_suppressed_includes_root_and_descendants() {
        let (world, _, root) = make_world_with_linear_causal_chain();
        let diff = counterfactual_replay(&world, vec![root]);
        // Chain has 3 changes (0, 1, 2). Root=0 suppresses 0, 1, 2.
        assert!(diff.suppressed_changes.contains(&root));
        assert!(diff.suppressed_changes.contains(&ChangeId(1)));
        assert!(diff.suppressed_changes.contains(&ChangeId(2)));
    }

    #[test]
    fn replay_divergence_batch_is_earliest_suppressed() {
        let (world, _, root) = make_world_with_linear_causal_chain();
        let diff = counterfactual_replay(&world, vec![root]);
        // All changes are in batch 1 in our test fixture.
        assert_eq!(diff.divergence_batch, Some(BatchId(1)));
    }

    #[test]
    fn replay_unrelated_root_does_not_suppress_relationship() {
        let (world, rel_id, _) = make_world_with_linear_causal_chain();
        // ChangeId(2) is downstream — it has no further descendants beyond itself.
        // Its removal does not cause the relationship (which was created by ChangeId(1)).
        // The relationship created_by = ChangeId(1); ChangeId(2) is a descendant of
        // ChangeId(1), not its ancestor. So removing ChangeId(2) alone should NOT
        // make the relationship absent.
        let diff = counterfactual_replay(&world, vec![ChangeId(2)]);
        assert!(
            !diff.absent_relationships.contains(&rel_id),
            "downstream-only root should not suppress the relationship"
        );
    }
}
