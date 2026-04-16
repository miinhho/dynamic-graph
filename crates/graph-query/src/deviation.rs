//! Entity deviation detection.
//!
//! Answers the question: *"how has this entity changed since batch B?"*
//!
//! The engine's sediment layers record every significant structural transition
//! an entity has gone through. This module walks those layers to surface
//! *deviations* — meaningful changes in membership, coherence, or lifecycle
//! state — since a caller-supplied baseline batch.
//!
//! ## Usage
//!
//! ```ignore
//! let baseline = BatchId(50);
//! let changes = graph_query::entity_deviations_since(&world, baseline);
//! for (entity_id, diff) in &changes {
//!     if diff.coherence_delta.abs() > 0.2 {
//!         println!("Entity {:?} coherence changed by {:.3}", entity_id, diff.coherence_delta);
//!     }
//! }
//! ```
//!
//! ## What counts as a deviation?
//!
//! A deviation is recorded when any of the following happened after `baseline`:
//! - **Born**: the entity did not exist at the baseline batch.
//! - **Became dormant**: the entity went dormant after the baseline.
//! - **Revived**: the entity revived from dormancy after the baseline.
//! - **Membership delta**: loci joined or left the entity.
//! - **Coherence shift**: coherence score changed significantly.
//!
//! Layers that were weathered away (Compressed/Skeleton) still contribute
//! summary info (coherence, member count) but cannot supply the full member
//! delta lists.

use graph_core::{
    BatchId, CompressionLevel, CompressedTransition, Entity, EntityId, EntityLayer,
    LayerTransition, LocusId,
};
use graph_world::World;

// ─── Output types ─────────────────────────────────────────────────────────────

/// A summary of how an entity deviated from its state at the baseline batch.
#[derive(Debug, Clone)]
pub struct EntityDiff {
    /// Entity id.
    pub entity_id: EntityId,

    /// True if the entity was born after the baseline.
    pub born_after_baseline: bool,

    /// True if the entity became dormant after the baseline.
    pub went_dormant: bool,

    /// True if the entity revived from dormancy after the baseline.
    pub revived: bool,

    /// Loci that joined the entity after the baseline (may be incomplete if
    /// relevant layers were weathered to Compressed or Skeleton).
    pub members_added: Vec<LocusId>,

    /// Loci that left the entity after the baseline (may be incomplete if
    /// relevant layers were weathered to Compressed or Skeleton).
    pub members_removed: Vec<LocusId>,

    /// Number of membership-delta events since baseline (includes weathered
    /// layers where the member lists are no longer available).
    pub membership_event_count: u32,

    /// Coherence score at the baseline batch (or the entity's birth coherence
    /// if it was born after the baseline).
    pub coherence_at_baseline: f32,

    /// Current coherence score.
    pub coherence_now: f32,

    /// `coherence_now - coherence_at_baseline`.
    pub coherence_delta: f32,

    /// Net change in member count since baseline (positive = grew, negative =
    /// shrank).
    pub member_count_delta: i64,

    /// The batch of the most recent layer deposited after the baseline.
    pub latest_change_batch: Option<BatchId>,
}

impl EntityDiff {
    /// True if *any* change was detected since the baseline.
    pub fn has_changes(&self) -> bool {
        self.born_after_baseline
            || self.went_dormant
            || self.revived
            || !self.members_added.is_empty()
            || !self.members_removed.is_empty()
            || self.membership_event_count > 0
            || self.coherence_delta.abs() > 1e-6
    }
}

// ─── Core function ────────────────────────────────────────────────────────────

/// Compute deviations for **all** entities since `baseline_batch`.
///
/// Returns one [`EntityDiff`] per entity that *has* changes since the
/// baseline. Entities with no layers after the baseline are excluded.
///
/// **Note**: "since baseline" means layers whose `batch > baseline_batch`.
/// Layers deposited exactly at the baseline are treated as pre-existing state.
pub fn entity_deviations_since(world: &World, baseline_batch: BatchId) -> Vec<EntityDiff> {
    world
        .entities()
        .iter()
        .filter_map(|entity| {
            let diff = diff_entity(entity, baseline_batch);
            if diff.has_changes() { Some(diff) } else { None }
        })
        .collect()
}

/// Compute the deviation for a single entity since `baseline_batch`.
///
/// Always returns an [`EntityDiff`]. Check [`EntityDiff::has_changes`] to
/// determine whether anything changed.
pub fn entity_diff(world: &World, entity_id: EntityId, baseline_batch: BatchId) -> Option<EntityDiff> {
    world.entities().get(entity_id).map(|e| diff_entity(e, baseline_batch))
}

// ─── Internal ─────────────────────────────────────────────────────────────────

fn diff_entity(entity: &Entity, baseline: BatchId) -> EntityDiff {
    // Coherence at or just before the baseline: walk layers in order (oldest
    // first) and take the last full-detail coherence we find at or before baseline.
    let coherence_at_baseline = baseline_coherence(entity, baseline);
    let member_count_at_baseline = baseline_member_count(entity, baseline);

    let mut born_after = false;
    let mut went_dormant = false;
    let mut revived = false;
    let mut members_added: Vec<LocusId> = Vec::new();
    let mut members_removed: Vec<LocusId> = Vec::new();
    let mut membership_event_count: u32 = 0;
    let mut latest_change_batch: Option<BatchId> = None;

    for layer in &entity.layers {
        if layer.batch <= baseline {
            continue;
        }

        // Track the latest batch that had a change.
        match latest_change_batch {
            None => latest_change_batch = Some(layer.batch),
            Some(prev) if layer.batch > prev => latest_change_batch = Some(layer.batch),
            _ => {}
        }

        match &layer.compression {
            CompressionLevel::Full => {
                // Full detail — examine the transition.
                classify_transition_full(
                    layer,
                    &mut born_after,
                    &mut went_dormant,
                    &mut revived,
                    &mut members_added,
                    &mut members_removed,
                    &mut membership_event_count,
                );
            }
            CompressionLevel::Compressed { transition_kind, .. }
            | CompressionLevel::Skeleton { transition_kind, .. } => {
                // Member lists not available; count the event only.
                classify_transition_compressed(
                    *transition_kind,
                    &mut born_after,
                    &mut went_dormant,
                    &mut revived,
                    &mut membership_event_count,
                );
            }
        }
    }

    let coherence_now = entity.current.coherence;
    let coherence_delta = coherence_now - coherence_at_baseline;

    let member_count_now = entity.current.members.len() as i64;
    let member_count_delta = member_count_now - member_count_at_baseline;

    EntityDiff {
        entity_id: entity.id,
        born_after_baseline: born_after,
        went_dormant,
        revived,
        members_added,
        members_removed,
        membership_event_count,
        coherence_at_baseline,
        coherence_now,
        coherence_delta,
        member_count_delta,
        latest_change_batch,
    }
}

/// Coherence score at or just before the baseline. Falls back to the birth
/// coherence if the entity was born after the baseline (returns 0.0 so the
/// full current coherence shows up as a delta).
fn baseline_coherence(entity: &Entity, baseline: BatchId) -> f32 {
    let mut last = 0.0f32;
    for layer in &entity.layers {
        if layer.batch > baseline {
            break;
        }
        let c = match &layer.compression {
            CompressionLevel::Full => {
                layer.snapshot.as_ref().map(|s| s.coherence).unwrap_or(last)
            }
            CompressionLevel::Compressed { coherence, .. }
            | CompressionLevel::Skeleton { coherence, .. } => *coherence,
        };
        last = c;
    }
    last
}

/// Member count at or just before the baseline.
fn baseline_member_count(entity: &Entity, baseline: BatchId) -> i64 {
    let mut last: i64 = 0;
    for layer in &entity.layers {
        if layer.batch > baseline {
            break;
        }
        let n = match &layer.compression {
            CompressionLevel::Full => {
                layer.snapshot.as_ref().map(|s| s.members.len() as i64).unwrap_or(last)
            }
            CompressionLevel::Compressed { member_count, .. }
            | CompressionLevel::Skeleton { member_count, .. } => *member_count as i64,
        };
        last = n;
    }
    last
}

fn classify_transition_full(
    layer: &EntityLayer,
    born_after: &mut bool,
    went_dormant: &mut bool,
    revived: &mut bool,
    members_added: &mut Vec<LocusId>,
    members_removed: &mut Vec<LocusId>,
    membership_event_count: &mut u32,
) {
    match &layer.transition {
        LayerTransition::Born => {
            *born_after = true;
        }
        LayerTransition::BecameDormant => {
            *went_dormant = true;
        }
        LayerTransition::Revived => {
            *revived = true;
        }
        LayerTransition::MembershipDelta { added, removed } => {
            *membership_event_count += 1;
            members_added.extend_from_slice(added);
            members_removed.extend_from_slice(removed);
        }
        LayerTransition::CoherenceShift { .. } => {
            // Coherence shift is captured via delta, not a separate field.
        }
        LayerTransition::Split { .. } | LayerTransition::Merged { .. } => {
            // Structural events — membership content is in the layer's
            // snapshot which the caller can inspect separately.
            *membership_event_count += 1;
        }
    }
}

fn classify_transition_compressed(
    kind: CompressedTransition,
    born_after: &mut bool,
    went_dormant: &mut bool,
    revived: &mut bool,
    membership_event_count: &mut u32,
) {
    match kind {
        CompressedTransition::Born => *born_after = true,
        CompressedTransition::BecameDormant => *went_dormant = true,
        CompressedTransition::Revived => *revived = true,
        CompressedTransition::MembershipDelta
        | CompressedTransition::Split
        | CompressedTransition::Merged => *membership_event_count += 1,
        CompressedTransition::CoherenceShift => {}
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{
        BatchId, Entity, EntityId, EntitySnapshot, EntityStatus, LayerTransition, LocusId,
    };
    use graph_world::World;

    fn snapshot(members: Vec<u64>, coherence: f32) -> EntitySnapshot {
        EntitySnapshot {
            members: members.into_iter().map(LocusId).collect(),
            member_relationships: vec![],
            coherence,
        }
    }

    fn make_entity_born_at(id: u64, batch: u64, coherence: f32) -> Entity {
        Entity::born(EntityId(id), BatchId(batch), snapshot(vec![1, 2], coherence))
    }

    #[test]
    fn no_changes_after_baseline_has_no_diff() {
        let mut world = World::new();
        world.entities_mut().insert(make_entity_born_at(0, 5, 0.8));
        // baseline is after the birth
        let diffs = entity_deviations_since(&world, BatchId(10));
        assert!(diffs.is_empty(), "entity born before baseline should not appear");
    }

    #[test]
    fn entity_born_after_baseline_detected() {
        let mut world = World::new();
        world.entities_mut().insert(make_entity_born_at(0, 15, 0.8));
        let diffs = entity_deviations_since(&world, BatchId(10));
        assert_eq!(diffs.len(), 1);
        assert!(diffs[0].born_after_baseline);
        assert!((diffs[0].coherence_now - 0.8).abs() < 1e-5);
    }

    #[test]
    fn membership_delta_captured() {
        let mut world = World::new();
        let mut entity = make_entity_born_at(0, 5, 0.5);
        entity.deposit(
            BatchId(12),
            snapshot(vec![1, 2, 3], 0.6),
            LayerTransition::MembershipDelta {
                added: vec![LocusId(3)],
                removed: vec![],
            },
        );
        world.entities_mut().insert(entity);

        let diffs = entity_deviations_since(&world, BatchId(10));
        assert_eq!(diffs.len(), 1);
        let d = &diffs[0];
        assert!(!d.born_after_baseline);
        assert_eq!(d.membership_event_count, 1);
        assert!(d.members_added.contains(&LocusId(3)));
        assert_eq!(d.member_count_delta, 1);
    }

    #[test]
    fn went_dormant_detected() {
        let mut world = World::new();
        let mut entity = make_entity_born_at(0, 5, 0.8);
        entity.deposit(BatchId(15), snapshot(vec![1, 2], 0.1), LayerTransition::BecameDormant);
        entity.status = EntityStatus::Dormant;
        world.entities_mut().insert(entity);

        let diffs = entity_deviations_since(&world, BatchId(10));
        assert_eq!(diffs.len(), 1);
        assert!(diffs[0].went_dormant);
        assert!(diffs[0].coherence_delta < 0.0);
    }

    #[test]
    fn coherence_shift_captured_in_delta() {
        let mut world = World::new();
        let mut entity = make_entity_born_at(0, 5, 0.4);
        entity.deposit(
            BatchId(12),
            snapshot(vec![1, 2], 0.9),
            LayerTransition::CoherenceShift { from: 0.4, to: 0.9 },
        );
        world.entities_mut().insert(entity);

        let diffs = entity_deviations_since(&world, BatchId(10));
        assert_eq!(diffs.len(), 1);
        let d = &diffs[0];
        assert!((d.coherence_at_baseline - 0.4).abs() < 1e-5, "baseline={}", d.coherence_at_baseline);
        assert!((d.coherence_now - 0.9).abs() < 1e-5);
        assert!((d.coherence_delta - 0.5).abs() < 1e-4, "delta={}", d.coherence_delta);
    }

    #[test]
    fn entity_diff_single_entity() {
        let mut world = World::new();
        world.entities_mut().insert(make_entity_born_at(42, 15, 0.7));
        let diff = entity_diff(&world, EntityId(42), BatchId(10)).unwrap();
        assert!(diff.born_after_baseline);
        let none = entity_diff(&world, EntityId(99), BatchId(10));
        assert!(none.is_none());
    }
}
