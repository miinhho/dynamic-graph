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
//! - Subscriptions: O(A) where A = total audit log length. Call
//!   `SubscriptionStore::trim_audit_before` alongside `ChangeLog::trim_before_batch`
//!   to keep this O(recent events).
//!
//! For large worlds with long histories, consider narrowing the range.

use graph_core::{BatchId, ChangeId, ChangeSubject, EntityId, LocusId, RelationshipId};
use rustc_hash::FxHashMap;

/// Activity delta for a relationship over a `WorldDiff` batch range.
///
/// Computed only from explicit `ChangeSubject::Relationship` log entries.
/// Relationships whose activity changed solely via auto-emergence do not
/// produce a `RelationshipDelta` — they appear in `relationships_updated`
/// only.
#[derive(Debug, Clone)]
pub struct RelationshipDelta {
    pub id: RelationshipId,
    /// Activity (slot 0) from the `before` state of the **first** explicit
    /// relationship change in the range.
    pub activity_before: f32,
    /// Activity (slot 0) from the `after` state of the **last** explicit
    /// relationship change in the range.
    pub activity_after: f32,
}

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

    // ── subscription layer ────────────────────────────────────────────────
    /// (subscriber, rel_id) pairs for new subscriptions registered in the range.
    pub subscriptions_added: Vec<(LocusId, RelationshipId)>,
    /// (subscriber, rel_id) pairs for subscriptions cancelled in the range.
    pub subscriptions_removed: Vec<(LocusId, RelationshipId)>,

    // ── relationship trajectory ───────────────────────────────────────────
    /// Relationships whose activity **increased** over the range, computed
    /// from explicit `ChangeSubject::Relationship` log entries. The delta
    /// reflects the first `before` and last `after` activity in the range.
    pub relationships_strengthening: Vec<RelationshipDelta>,
    /// Relationships whose activity **decreased** over the range (same
    /// semantics as `relationships_strengthening`).
    pub relationships_weakening: Vec<RelationshipDelta>,

    // ── pruning ───────────────────────────────────────────────────────────
    /// Relationships pruned (activity below threshold) during the range.
    /// These IDs no longer exist in the world — this field is the only
    /// record of their removal.
    pub relationships_pruned: Vec<RelationshipId>,
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
            && self.subscriptions_added.is_empty()
            && self.subscriptions_removed.is_empty()
            && self.relationships_strengthening.is_empty()
            && self.relationships_weakening.is_empty()
            && self.relationships_pruned.is_empty()
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

        // ── changes + relationship trajectory ─────────────────────────────
        let mut change_ids: Vec<ChangeId> = Vec::new();
        // rel_id → (activity_before_range, activity_after_range)
        // Populated lazily — allocated only if the range contains at least
        // one ChangeSubject::Relationship entry (uncommon in pure locus workloads).
        let mut rel_activity_range: Option<FxHashMap<RelationshipId, (f32, f32)>> = None;

        for b in from.0..to.0 {
            for c in world.log().batch(BatchId(b)) {
                change_ids.push(c.id);
                if let ChangeSubject::Relationship(rel_id) = c.subject {
                    let before_act = c.before.as_slice().first().copied().unwrap_or(0.0);
                    let after_act = c.after.as_slice().first().copied().unwrap_or(0.0);
                    rel_activity_range
                        .get_or_insert_with(FxHashMap::default)
                        .entry(rel_id)
                        .and_modify(|r| r.1 = after_act)
                        .or_insert((before_act, after_act));
                }
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

        // ── subscriptions ─────────────────────────────────────────────────
        let mut subscriptions_added = Vec::new();
        let mut subscriptions_removed = Vec::new();
        for event in world.subscriptions().events_in_range(from, to) {
            if event.subscribed {
                subscriptions_added.push((event.subscriber, event.rel_id));
            } else {
                subscriptions_removed.push((event.subscriber, event.rel_id));
            }
        }

        // ── relationship trajectory ───────────────────────────────────────
        let mut relationships_strengthening = Vec::new();
        let mut relationships_weakening = Vec::new();
        for (id, (activity_before, activity_after)) in rel_activity_range.unwrap_or_default() {
            let delta = RelationshipDelta { id, activity_before, activity_after };
            if activity_after > activity_before {
                relationships_strengthening.push(delta);
            } else if activity_after < activity_before {
                relationships_weakening.push(delta);
            }
            // Equal: no trajectory signal — skip.
        }

        // ── pruning ───────────────────────────────────────────────────────
        let relationships_pruned: Vec<RelationshipId> = world
            .pruned_log()
            .iter()
            .filter(|(_, b)| b.0 >= from.0 && b.0 < to.0)
            .map(|(id, _)| *id)
            .collect();

        WorldDiff {
            from_batch: from,
            to_batch: to,
            change_ids,
            relationships_created,
            relationships_updated,
            entities_changed,
            subscriptions_added,
            subscriptions_removed,
            relationships_strengthening,
            relationships_weakening,
            relationships_pruned,
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
            wall_time: None,
            metadata: None,
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

    #[test]
    fn diff_captures_subscription_events() {
        let mut w = two_locus_world();
        let locus = LocusId(0);

        // Manufacture a relationship id without actually inserting one — we just
        // need something to subscribe to for this test.
        use graph_core::RelationshipId;
        let rel = RelationshipId(42);

        let before = w.current_batch(); // batch 0
        // Subscribe at batch 0.
        w.subscriptions_mut().subscribe_at(locus, rel, Some(before));
        w.advance_batch(); // batch 1

        let diff = w.diff_between(before, w.current_batch());
        assert_eq!(diff.subscriptions_added, vec![(locus, rel)]);
        assert!(diff.subscriptions_removed.is_empty());
    }

    #[test]
    fn diff_captures_unsubscribe_event() {
        let mut w = two_locus_world();
        let locus = LocusId(0);
        use graph_core::RelationshipId;
        let rel = RelationshipId(42);

        // Subscribe before the window we care about.
        w.subscriptions_mut().subscribe_at(locus, rel, None);

        w.advance_batch(); // batch 1
        let from = w.current_batch();
        // Unsubscribe inside the window.
        w.subscriptions_mut().unsubscribe_at(locus, rel, Some(from));
        w.advance_batch(); // batch 2

        let diff = w.diff_between(from, w.current_batch());
        assert!(diff.subscriptions_added.is_empty());
        assert_eq!(diff.subscriptions_removed, vec![(locus, rel)]);
    }

    #[test]
    fn diff_excludes_subscription_events_outside_range() {
        let mut w = two_locus_world();
        let locus = LocusId(0);
        use graph_core::RelationshipId;
        let rel_a = RelationshipId(10);
        let rel_b = RelationshipId(11);

        let batch0 = w.current_batch(); // batch 0
        w.subscriptions_mut().subscribe_at(locus, rel_a, Some(batch0));
        w.advance_batch(); // batch 1
        let batch1 = w.current_batch();
        w.subscriptions_mut().subscribe_at(locus, rel_b, Some(batch1));
        w.advance_batch(); // batch 2

        // Only look at batch 1 (exclusive of batch 0 event).
        let diff = w.diff_between(batch1, w.current_batch());
        assert_eq!(diff.subscriptions_added, vec![(locus, rel_b)]);
    }

    // ── relationship trajectory ──────────────────────────────────────────────

    fn append_rel_change(world: &mut World, rel_id: graph_core::RelationshipId, activity: f32) -> ChangeId {
        use graph_core::{Change, StateVector as SV};
        let id = world.mint_change_id();
        let batch = world.current_batch();
        let old_act = world.relationships().get(rel_id)
            .map(|r| r.state.as_slice()[0])
            .unwrap_or(0.0);
        world.append_change(Change {
            id,
            subject: ChangeSubject::Relationship(rel_id),
            kind: InfluenceKindId(1),
            predecessors: vec![],
            before: SV::from_slice(&[old_act, 0.0]),
            after: SV::from_slice(&[activity, 0.0]),
            batch,
            wall_time: None,
            metadata: None,
        });
        id
    }

    fn world_with_relationship() -> (World, graph_core::RelationshipId) {
        use graph_core::{
            Endpoints, InfluenceKindId as IKId, Relationship, RelationshipKindId,
            RelationshipLineage, StateVector as SV,
        };
        let rk: RelationshipKindId = IKId(1);
        let mut w = two_locus_world();
        let rel_id = w.relationships_mut().mint_id();
        w.relationships_mut().insert(Relationship {
            id: rel_id,
            kind: rk,
            endpoints: Endpoints::Directed { from: LocusId(0), to: LocusId(1) },
            state: SV::from_slice(&[0.5, 0.0]),
            lineage: RelationshipLineage {
                created_by: None,
                last_touched_by: None,
                change_count: 0,
                kinds_observed: vec![rk],
            },
            last_decayed_batch: 0,
        });
        (w, rel_id)
    }

    #[test]
    fn diff_trajectory_strengthening_detected() {
        let (mut w, rel_id) = world_with_relationship();
        let from = w.current_batch();
        append_rel_change(&mut w, rel_id, 0.9); // activity 0.5 → 0.9
        w.advance_batch();
        let diff = w.diff_between(from, w.current_batch());

        assert_eq!(diff.relationships_strengthening.len(), 1);
        assert!(diff.relationships_weakening.is_empty());
        let delta = &diff.relationships_strengthening[0];
        assert_eq!(delta.id, rel_id);
        assert!((delta.activity_before - 0.5).abs() < 1e-5);
        assert!((delta.activity_after - 0.9).abs() < 1e-5);
    }

    #[test]
    fn diff_trajectory_weakening_detected() {
        let (mut w, rel_id) = world_with_relationship();
        let from = w.current_batch();
        append_rel_change(&mut w, rel_id, 0.1); // activity 0.5 → 0.1
        w.advance_batch();
        let diff = w.diff_between(from, w.current_batch());

        assert_eq!(diff.relationships_weakening.len(), 1);
        assert!(diff.relationships_strengthening.is_empty());
    }

    #[test]
    fn diff_trajectory_uses_first_before_and_last_after() {
        // Multiple changes in range: first.before → last.after determines trajectory.
        let (mut w, rel_id) = world_with_relationship();
        let from = w.current_batch();
        // 0.5 → 0.8 → 0.3 → 0.7: net is strengthening (0.5 → 0.7)
        for &act in &[0.8f32, 0.3, 0.7] {
            append_rel_change(&mut w, rel_id, act);
        }
        w.advance_batch();
        let diff = w.diff_between(from, w.current_batch());

        // Net: 0.5 before, 0.7 after → strengthening
        assert_eq!(diff.relationships_strengthening.len(), 1);
        let delta = &diff.relationships_strengthening[0];
        assert!((delta.activity_before - 0.5).abs() < 1e-5, "before={}", delta.activity_before);
        assert!((delta.activity_after - 0.7).abs() < 1e-5, "after={}", delta.activity_after);
    }

    // ── pruning log ──────────────────────────────────────────────────────────

    #[test]
    fn diff_includes_pruned_relationships_in_range() {
        use graph_core::RelationshipId;
        let mut w = two_locus_world();
        let from = w.current_batch(); // batch 0
        w.record_pruned(RelationshipId(42));
        w.advance_batch(); // batch 1
        let diff = w.diff_between(from, w.current_batch());
        assert_eq!(diff.relationships_pruned, vec![RelationshipId(42)]);
    }

    #[test]
    fn diff_excludes_pruned_outside_range() {
        use graph_core::RelationshipId;
        let mut w = two_locus_world();
        w.record_pruned(RelationshipId(1)); // batch 0
        w.advance_batch();                  // batch 1
        let from = w.current_batch();
        // No pruning in batch 1
        w.advance_batch();
        let diff = w.diff_between(from, w.current_batch());
        assert!(diff.relationships_pruned.is_empty());
    }

    #[test]
    fn trim_pruned_log_clears_old_entries() {
        use graph_core::RelationshipId;
        let mut w = two_locus_world();
        w.record_pruned(RelationshipId(1));
        w.advance_batch();
        w.record_pruned(RelationshipId(2));

        // Trim entries before batch 1 — removes entry for RelationshipId(1).
        w.trim_pruned_log_before(BatchId(1));
        assert_eq!(w.pruned_log().len(), 1);
        assert_eq!(w.pruned_log()[0].0, RelationshipId(2));
    }
}
