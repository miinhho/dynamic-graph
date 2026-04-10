//! Batch-range diff: what changed between two `BatchId` snapshots.
//!
//! `WorldDiff` captures a summary of activity between two batch indices
//! so callers can answer questions like "which relationships emerged in
//! the last 10 batches?" without manually iterating every store.
//!
//! ## Usage
//!
//! ```ignore
//! let before = world.current_batch();
//! sim.step_n(10, vec![stimulus(1.0)]);
//! let diff = world.diff_since(before);
//! println!("{} changes, {} new relationships", diff.change_count(), diff.relationships_created.len());
//! ```
//!
//! ## Batch ranges
//!
//! `from_batch` is **inclusive**; `to_batch` is **exclusive** (following
//! the standard Rust convention for ranges). So a diff produced just
//! after a tick where `prev_batch = 3` and `world.current_batch() = 7`
//! covers batches 3, 4, 5, 6.
//!
//! ## What is and isn't tracked
//!
//! `WorldDiff` tracks state changes that are represented as `Change` events
//! in the `ChangeLog`. Concretely:
//!
//! - **Locus state changes**: every `ProposedChange` committed by the batch
//!   loop produces a `Change` event → always tracked.
//! - **Relationship auto-emergence**: when cross-locus predecessors trigger
//!   `ChangeSubject::Relationship` events, these are recorded in the log →
//!   `relationships_created` / `relationships_updated` reflect them.
//! - **Hebbian weight updates**: Hebbian learning always co-occurs with
//!   auto-emergence (the same batch that observes `pre × post` also fires
//!   the relationship change event that sets `last_touched_by`) → tracked
//!   via `relationships_updated`.
//! - **Lazy activity decay**: the engine applies per-kind decay by directly
//!   mutating relationship state without producing `Change` events. Decay is
//!   therefore **not** reflected in `WorldDiff`. This is correct by design —
//!   decay is a background erosion process, not an observable event.
//!
//! ## Complexity
//!
//! - Changes: O(k) where k = changes in the range (via `by_batch` index).
//! - Relationships: O(R) — iterates all relationships and checks lineage.
//! - Entities: O(E × L_avg) — iterates layers per entity.
//!
//! For large worlds with long histories, consider narrowing the range.

use graph_core::{BatchId, ChangeId, EntityId, RelationshipId};

/// Summary of world state changes between `from_batch` (inclusive) and
/// `to_batch` (exclusive).
#[derive(Debug, Clone, Default)]
pub struct WorldDiff {
    /// The batch range covered: `from_batch..to_batch`.
    pub from_batch: BatchId,
    pub to_batch: BatchId,

    // ── change log ────────────────────────────────────────────────────────
    /// IDs of all changes committed in `[from_batch, to_batch)`.
    pub change_ids: Vec<ChangeId>,

    // ── relationship layer ────────────────────────────────────────────────
    /// Relationships whose `lineage.created_by` change falls in the range.
    pub relationships_created: Vec<RelationshipId>,
    /// Relationships that were touched (but not created) in the range:
    /// `lineage.last_touched_by` change is in range and `created_by` is not.
    ///
    /// This includes relationships whose activity was bumped by auto-emergence
    /// AND relationships whose weight was updated by Hebbian learning (both
    /// always co-occur with a `ChangeSubject::Relationship` event). It does
    /// **not** include relationships that changed only due to lazy decay, since
    /// decay does not produce Change events.
    pub relationships_updated: Vec<RelationshipId>,

    // ── entity layer ──────────────────────────────────────────────────────
    /// Entities that deposited at least one layer in the range. Includes
    /// newly born entities and entities that received a continuation layer.
    pub entities_changed: Vec<EntityId>,
}

impl WorldDiff {
    /// Number of changes in the diff.
    #[inline]
    pub fn change_count(&self) -> usize {
        self.change_ids.len()
    }

    /// `true` if nothing changed in the covered range.
    pub fn is_empty(&self) -> bool {
        self.change_ids.is_empty()
            && self.relationships_created.is_empty()
            && self.relationships_updated.is_empty()
            && self.entities_changed.is_empty()
    }

    /// Build a `WorldDiff` for `[from, to)`.
    pub(crate) fn compute(world: &crate::world::World, from: BatchId, to: BatchId) -> Self {
        if from.0 >= to.0 {
            return WorldDiff {
                from_batch: from,
                to_batch: to,
                ..Default::default()
            };
        }

        // ── changes ───────────────────────────────────────────────────────
        let mut change_ids: Vec<ChangeId> = Vec::new();
        for b in from.0..to.0 {
            for c in world.log().batch(BatchId(b)) {
                change_ids.push(c.id);
            }
        }

        // Build a set of change ids in range for O(1) membership tests below.
        let in_range: rustc_hash::FxHashSet<ChangeId> = change_ids.iter().copied().collect();

        // ── relationships ─────────────────────────────────────────────────
        let mut relationships_created = Vec::new();
        let mut relationships_updated = Vec::new();
        for rel in world.relationships().iter() {
            let created_in_range = rel.lineage.created_by
                .map(|cid| in_range.contains(&cid))
                .unwrap_or(false);
            let touched_in_range = rel.lineage.last_touched_by
                .map(|cid| in_range.contains(&cid))
                .unwrap_or(false);

            if created_in_range {
                relationships_created.push(rel.id);
            } else if touched_in_range {
                relationships_updated.push(rel.id);
            }
        }

        // ── entities ──────────────────────────────────────────────────────
        let mut entities_changed = Vec::new();
        for entity in world.entities().iter() {
            let has_layer_in_range = entity
                .layers
                .iter()
                .any(|l| l.batch.0 >= from.0 && l.batch.0 < to.0);
            if has_layer_in_range {
                entities_changed.push(entity.id);
            }
        }

        WorldDiff {
            from_batch: from,
            to_batch: to,
            change_ids,
            relationships_created,
            relationships_updated,
            entities_changed,
        }
    }
}

#[cfg(test)]
mod tests {
    use graph_core::{BatchId, ChangeId, ChangeSubject, InfluenceKindId, Locus, LocusId,
                     LocusKindId, StateVector};
    use crate::world::World;

    fn two_locus_world() -> World {
        let mut w = World::new();
        w.insert_locus(Locus::new(LocusId(0), LocusKindId(1), StateVector::zeros(1)));
        w.insert_locus(Locus::new(LocusId(1), LocusKindId(1), StateVector::zeros(1)));
        w
    }

    fn append_change(world: &mut World, locus: LocusId) -> ChangeId {
        use graph_core::{Change, StateVector as SV};
        let id = world.mint_change_id();
        let batch = world.current_batch();
        world.append_change(Change {
            id,
            subject: ChangeSubject::Locus(locus),
            kind: InfluenceKindId(1),
            predecessors: vec![],
            before: SV::zeros(1),
            after: SV::from_slice(&[1.0]),
            batch,
        });
        id
    }

    #[test]
    fn empty_range_produces_empty_diff() {
        let w = two_locus_world();
        let diff = w.diff_since(w.current_batch());
        assert!(diff.is_empty());
    }

    #[test]
    fn diff_captures_changes_in_range() {
        let mut w = two_locus_world();
        let before = w.current_batch(); // batch 0
        let cid = append_change(&mut w, LocusId(0));
        w.advance_batch(); // batch 1
        append_change(&mut w, LocusId(1));
        // diff [0, 1) should contain only the first change
        let diff = w.diff_between(before, BatchId(1));
        assert_eq!(diff.change_ids, vec![cid]);
        assert_eq!(diff.from_batch, BatchId(0));
        assert_eq!(diff.to_batch, BatchId(1));
    }

    #[test]
    fn diff_excludes_changes_before_from_batch() {
        let mut w = two_locus_world();
        append_change(&mut w, LocusId(0)); // batch 0
        w.advance_batch();
        let from = w.current_batch(); // batch 1
        let cid = append_change(&mut w, LocusId(1)); // batch 1
        w.advance_batch(); // batch 2
        let diff = w.diff_since(from);
        // should see only the batch-1 change
        assert_eq!(diff.change_ids, vec![cid]);
    }

    #[test]
    fn from_ge_to_returns_empty_diff() {
        let w = two_locus_world();
        let diff = w.diff_between(BatchId(5), BatchId(3));
        assert!(diff.is_empty());
    }
}
