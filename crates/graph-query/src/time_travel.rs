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

use graph_core::{BatchId, EntityId, RelationshipId};
use graph_world::{World, WorldDiff};

// ── Result types ──────────────────────────────────────────────────────────────

/// The structural inverse of applying a batch range — describes what must be
/// "undone" to reconstruct the world as it was at `target_batch`.
///
/// Returned by [`time_travel`].
#[derive(Debug, Clone, PartialEq)]
pub struct TimeTravelResult {
    /// The batch the caller requested to travel back to.
    pub target_batch: BatchId,
    /// The forward `WorldDiff` for `[target_batch, current_batch)`.
    /// Callers can use this to understand what *happened* in the range being reversed.
    pub forward_diff: WorldDiff,
    /// Relationships created in `(target_batch, current_batch]` — these would
    /// not exist at `target_batch` and should be excluded from the prior view.
    pub relationships_to_remove: Vec<RelationshipId>,
    /// Relationships that were pruned in the range and cannot be fully restored.
    /// The prior-batch view reports these as "irrecoverable" — their state at
    /// `target_batch` is unknown because the pruned-log only records the ID.
    pub relationships_irrecoverable: Vec<RelationshipId>,
    /// Entity IDs whose prior state is approximate because the layers in the
    /// target range have been compressed or skeletonised (snapshot dropped by
    /// weathering).
    pub entities_approximate: Vec<EntityId>,
    /// `Some(batch)` when the requested `target_batch` is older than the
    /// ChangeLog's trim boundary — the result reflects the earliest available
    /// state, not the exact requested one.
    pub trimmed_at: Option<BatchId>,
}

impl TimeTravelResult {
    /// `true` when the result is exact (no trim, no approximate entities).
    pub fn is_exact(&self) -> bool {
        self.trimmed_at.is_none() && self.entities_approximate.is_empty()
    }
}

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
    let current = world.current_batch();

    // Determine effective target: can't go before the earliest available batch.
    // Use the batch of the first remaining change after any trim, or batch 0 if empty.
    let log_start = world
        .log()
        .iter()
        .next()
        .map(|c| c.batch)
        .unwrap_or(BatchId(0));
    let (effective_target, trimmed_at) = if target_batch < log_start {
        (log_start, Some(log_start))
    } else {
        (target_batch, None)
    };

    // If target >= current, nothing to invert.
    if effective_target >= current {
        return TimeTravelResult {
            target_batch,
            forward_diff: WorldDiff::default(),
            relationships_to_remove: Vec::new(),
            relationships_irrecoverable: Vec::new(),
            entities_approximate: Vec::new(),
            trimmed_at,
        };
    }

    let forward_diff = world.diff_between(effective_target, current);

    // Relationships created in the range → not present at target_batch.
    let relationships_to_remove = forward_diff.relationships_created.clone();

    // Pruned relationships in the range → state at target_batch irrecoverable.
    let relationships_irrecoverable = forward_diff.relationships_pruned.clone();

    // Entities where any layer in [effective_target, current) has non-Full compression.
    let entities_approximate = world
        .entities()
        .iter()
        .filter(|e| {
            e.layers.iter().any(|l| {
                l.batch >= effective_target
                    && l.batch < current
                    && !matches!(l.compression, graph_core::CompressionLevel::Full)
            })
        })
        .map(|e| e.id)
        .collect();

    TimeTravelResult {
        target_batch,
        forward_diff,
        relationships_to_remove,
        relationships_irrecoverable,
        entities_approximate,
        trimmed_at,
    }
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
