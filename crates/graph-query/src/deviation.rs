mod analysis;
mod baseline;
mod observed;
mod types;

use graph_core::{BatchId, EntityId};
use graph_world::World;

use self::analysis::diff_entity;

pub use self::types::EntityDiff;

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
pub fn entity_diff(
    world: &World,
    entity_id: EntityId,
    baseline_batch: BatchId,
) -> Option<EntityDiff> {
    world
        .entities()
        .get(entity_id)
        .map(|e| diff_entity(e, baseline_batch))
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
        Entity::born(
            EntityId(id),
            BatchId(batch),
            snapshot(vec![1, 2], coherence),
        )
    }

    #[test]
    fn no_changes_after_baseline_has_no_diff() {
        let mut world = World::new();
        world.entities_mut().insert(make_entity_born_at(0, 5, 0.8));
        // baseline is after the birth
        let diffs = entity_deviations_since(&world, BatchId(10));
        assert!(
            diffs.is_empty(),
            "entity born before baseline should not appear"
        );
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
        entity.deposit(
            BatchId(15),
            snapshot(vec![1, 2], 0.1),
            LayerTransition::BecameDormant,
        );
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
        assert!(
            (d.coherence_at_baseline - 0.4).abs() < 1e-5,
            "baseline={}",
            d.coherence_at_baseline
        );
        assert!((d.coherence_now - 0.9).abs() < 1e-5);
        assert!(
            (d.coherence_delta - 0.5).abs() < 1e-4,
            "delta={}",
            d.coherence_delta
        );
    }

    #[test]
    fn entity_diff_single_entity() {
        let mut world = World::new();
        world
            .entities_mut()
            .insert(make_entity_born_at(42, 15, 0.7));
        let diff = entity_diff(&world, EntityId(42), BatchId(10)).unwrap();
        assert!(diff.born_after_baseline);
        let none = entity_diff(&world, EntityId(99), BatchId(10));
        assert!(none.is_none());
    }
}
