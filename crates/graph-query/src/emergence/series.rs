use std::collections::BTreeSet;

use graph_core::{BatchId, ChangeSubject, EntityId, LocusId, RelationshipId};
use graph_world::World;

use super::{DEFAULT_MIN_ACTIVITY_THRESHOLD, DecayRates, is_lifecycle_transition};

pub(super) fn coherence_dense_series_inner(
    world: &World,
    entity_id: EntityId,
    decay_rates: Option<&DecayRates>,
) -> Vec<(BatchId, f32)> {
    let Some(entity) = world.entities().get(entity_id) else {
        return Vec::new();
    };

    let members = &entity.current.members;
    let member_rels = &entity.current.member_relationships;
    if member_rels.is_empty() {
        return Vec::new();
    }

    sample_batches(world, entity, member_rels)
        .into_iter()
        .map(|batch| {
            let coherence = coherence_at_batch(
                world,
                members,
                member_rels,
                batch,
                DEFAULT_MIN_ACTIVITY_THRESHOLD,
                decay_rates,
            );
            (batch, coherence)
        })
        .collect()
}

pub(super) fn rel_weight_at(batch: BatchId, rel_id: RelationshipId, world: &World) -> f64 {
    world
        .changes_to_relationship(rel_id)
        .find(|change| {
            change.batch <= batch && matches!(change.subject, ChangeSubject::Relationship(_))
        })
        .and_then(|change| change.after.as_slice().get(1).copied())
        .unwrap_or(0.0) as f64
}

fn sample_batches(
    world: &World,
    entity: &graph_core::Entity,
    member_relationships: &[RelationshipId],
) -> BTreeSet<BatchId> {
    let window_start = entity
        .layers
        .iter()
        .rposition(is_lifecycle_transition)
        .map(|index| entity.layers[index].batch)
        .unwrap_or(BatchId(0));

    let mut batches = BTreeSet::new();
    for &relationship_id in member_relationships {
        for change in world.changes_to_relationship(relationship_id) {
            if change.batch > window_start {
                batches.insert(change.batch);
            }
        }
    }
    batches
}

pub(super) fn rel_activity_at(
    batch: BatchId,
    rel_id: RelationshipId,
    world: &World,
    decay_rates: Option<&DecayRates>,
) -> f32 {
    let Some(change) = world.changes_to_relationship(rel_id).find(|change| {
        change.batch <= batch && matches!(change.subject, ChangeSubject::Relationship(_))
    }) else {
        return 0.0;
    };

    let after_activity = change.after.as_slice().first().copied().unwrap_or(0.0);
    let Some(rates) = decay_rates else {
        return after_activity;
    };
    let Some(rel) = world.relationships().get(rel_id) else {
        return after_activity;
    };
    let rate = rates.get(&rel.kind).copied().unwrap_or(1.0);
    if rate >= 1.0 - f32::EPSILON {
        return after_activity;
    }

    let gap = batch.0.saturating_sub(change.batch.0);
    if gap == 0 {
        return after_activity;
    }
    after_activity * rate.powi(gap.min(i32::MAX as u64) as i32)
}

fn coherence_at_batch(
    world: &World,
    members: &[LocusId],
    member_rels: &[RelationshipId],
    batch: BatchId,
    threshold: f32,
    decay_rates: Option<&DecayRates>,
) -> f32 {
    let member_set: rustc_hash::FxHashSet<LocusId> = members.iter().copied().collect();
    let mut total_activity = 0.0;
    let mut active_count = 0usize;

    for &rel_id in member_rels {
        let Some(rel) = world.relationships().get(rel_id) else {
            continue;
        };
        if !rel.endpoints.all_endpoints_in(&member_set) {
            continue;
        }

        let activity = rel_activity_at(batch, rel_id, world, decay_rates);
        if activity >= threshold {
            total_activity += activity;
            active_count += 1;
        }
    }

    let mean_activity = if active_count == 0 {
        0.0
    } else {
        total_activity / active_count as f32
    };
    let density = (active_count as f32 / density_reference(members.len())).min(1.0);
    mean_activity * density
}

fn density_reference(member_count: usize) -> f32 {
    if member_count <= 1 {
        1.0
    } else {
        let n = member_count as f32;
        n * (n + 1.0).ln() / 2.0
    }
}
