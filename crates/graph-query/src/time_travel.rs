//! B3 — Time-travel queries via WorldDiff reverse replay.
//!
//! Answers "what did the world look like at batch N?" by inverting the
//! `WorldDiff` covering `[target_batch, current_batch)`.
//!
//! See `docs/b3-time-travel.md` for the full design rationale and semantics,
//! including limitations around compressed entity layers, pruned relationships,
//! and trimmed change logs.
//!
//! ## Usage
//!
//! ```ignore
//! let result = graph_query::time_travel(&world, BatchId(10));
//! if let Some(trim) = result.trimmed_at {
//!     println!("warn: ChangeLog trimmed before batch {:?}", trim);
//! }
//! for rel_id in &result.relationships_to_remove {
//!     println!("relationship {:?} did not exist at target batch", rel_id);
//! }
//! ```

mod analysis;
mod result;

use graph_core::BatchId;
use graph_world::World;

use self::analysis::build_time_travel_result;
pub use self::result::TimeTravelResult;

// ── Core function ─────────────────────────────────────────────────────────────

/// Compute the structural inverse of going from `current_batch` back to `target_batch`.
///
/// The algorithm:
/// 1. Clamp `target_batch` to the ChangeLog's earliest available batch. If
///    clamping was necessary, `result.trimmed_at` is set.
/// 2. Compute the forward `WorldDiff` for `[effective_target, current_batch)`.
/// 3. From the diff, extract:
///    - `relationships_to_remove`: IDs from `diff.relationships_created`
///    - `relationships_irrecoverable`: IDs from `diff.relationships_pruned`
///    - `entities_approximate`: entity IDs where any layer in the range has
///      non-Full compression
/// 4. Return the assembled `TimeTravelResult`.
///
/// **Complexity**: O(k + R + E·L_avg) — same as `WorldDiff::compute`.
pub fn time_travel(world: &World, target_batch: BatchId) -> TimeTravelResult {
    build_time_travel_result(world, target_batch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{
        BatchId, Change, ChangeId, ChangeSubject, Endpoints, InfluenceKindId, KindObservation,
        Locus, LocusId, LocusKindId, Relationship, RelationshipId, RelationshipLineage,
        StateVector,
    };
    use graph_world::World;

    fn two_locus_world() -> World {
        let mut w = World::new();
        w.insert_locus(Locus::new(
            LocusId(0),
            LocusKindId(1),
            StateVector::zeros(1),
        ));
        w.insert_locus(Locus::new(
            LocusId(1),
            LocusKindId(1),
            StateVector::zeros(1),
        ));
        w
    }

    fn append_change(world: &mut World, locus: LocusId) -> ChangeId {
        let id = world.mint_change_id();
        let batch = world.current_batch();
        world.append_change(Change {
            id,
            subject: ChangeSubject::Locus(locus),
            kind: InfluenceKindId(1),
            predecessors: vec![],
            before: StateVector::zeros(1),
            after: StateVector::from_slice(&[1.0]),
            batch,
            wall_time: None,
            metadata: None,
        });
        id
    }

    fn insert_relationship_created_by(world: &mut World, created_by: ChangeId) -> RelationshipId {
        let rel_id = world.relationships_mut().mint_id();
        let current_batch = world.current_batch();
        world.relationships_mut().insert(Relationship {
            id: rel_id,
            kind: InfluenceKindId(1),
            endpoints: Endpoints::Directed {
                from: LocusId(0),
                to: LocusId(1),
            },
            state: StateVector::from_slice(&[0.5, 0.0]),
            lineage: RelationshipLineage {
                created_by: Some(created_by),
                last_touched_by: Some(created_by),
                change_count: 1,
                kinds_observed: smallvec::smallvec![KindObservation::synthetic(InfluenceKindId(1))],
            },
            created_batch: current_batch,
            last_decayed_batch: 0,
            metadata: None,
        });
        rel_id
    }

    #[test]
    fn target_at_current_returns_empty_diff() {
        let w = two_locus_world();
        let result = time_travel(&w, w.current_batch());
        assert!(result.forward_diff.is_empty());
        assert!(result.relationships_to_remove.is_empty());
        assert!(result.trimmed_at.is_none());
    }

    #[test]
    fn target_after_current_returns_empty() {
        let w = two_locus_world();
        let future = BatchId(w.current_batch().0 + 100);
        let result = time_travel(&w, future);
        assert!(result.forward_diff.is_empty());
    }

    #[test]
    fn relationships_created_after_target_listed_to_remove() {
        let mut w = two_locus_world();
        let target = w.current_batch(); // batch 0

        // Advance to batch 1, create a change and relationship.
        w.advance_batch();
        let cid = append_change(&mut w, LocusId(0));
        let rel_id = insert_relationship_created_by(&mut w, cid);
        w.advance_batch();

        let result = time_travel(&w, target);
        assert!(
            result.relationships_to_remove.contains(&rel_id),
            "relationship created after target should be in to_remove"
        );
    }

    #[test]
    fn pruned_relationships_listed_as_irrecoverable() {
        let mut w = two_locus_world();
        let target = w.current_batch(); // batch 0

        w.record_pruned(RelationshipId(99));
        w.advance_batch();

        let result = time_travel(&w, target);
        assert!(
            result
                .relationships_irrecoverable
                .contains(&RelationshipId(99)),
            "pruned relationship should be irrecoverable"
        );
    }

    #[test]
    fn no_trim_returns_exact_result() {
        let mut w = two_locus_world();
        let target = w.current_batch();
        append_change(&mut w, LocusId(0));
        w.advance_batch();

        let result = time_travel(&w, target);
        assert!(
            result.trimmed_at.is_none(),
            "should be exact without trimming"
        );
        assert!(result.is_exact() || !result.entities_approximate.is_empty());
    }

    #[test]
    fn trimmed_log_sets_trimmed_at() {
        let mut w = two_locus_world();
        // Advance a few batches.
        for _ in 0..5 {
            append_change(&mut w, LocusId(0));
            w.advance_batch();
        }
        // Trim to batch 3 (retain from batch 3 onward).
        w.log_mut().trim_before_batch(BatchId(3));

        // Request batch 1 — earlier than trim boundary (batch 3).
        let result = time_travel(&w, BatchId(1));
        assert!(result.trimmed_at.is_some(), "should report trim boundary");
        assert!(
            result.trimmed_at.unwrap().0 >= 3,
            "trimmed_at should be at or after the trim boundary"
        );
    }
}
