use std::collections::HashSet;

use graph_core::{BatchId, ChangeId, ChangeSubject, EntityId, LocusId};
use graph_world::World;

use crate::causality::causal_ancestors;
use crate::entity_causality::{cause_seed_changes, entity_transition_cause};

pub fn entity_upstream_transitions(
    world: &World,
    entity_id: EntityId,
    at_batch: BatchId,
) -> Vec<(EntityId, BatchId)> {
    let Some(cause) = entity_transition_cause(world, entity_id, at_batch) else {
        return Vec::new();
    };

    let seeds = cause_seed_changes(world, &cause, at_batch);
    if seeds.is_empty() {
        return Vec::new();
    }

    let all_ancestors = collect_ancestor_change_ids(world, seeds);
    let ancestor_loci = collect_ancestor_loci(world, &all_ancestors);
    if ancestor_loci.is_empty() {
        return Vec::new();
    }

    collect_upstream_entity_transitions(world, entity_id, at_batch, &ancestor_loci)
}

pub(super) fn collect_ancestor_change_ids(
    world: &World,
    seeds: Vec<ChangeId>,
) -> HashSet<ChangeId> {
    let mut all_ancestors = HashSet::new();
    for seed in seeds {
        all_ancestors.insert(seed);
        for ancestor in causal_ancestors(world, seed) {
            all_ancestors.insert(ancestor);
        }
    }
    all_ancestors
}

pub(super) fn collect_ancestor_loci(
    world: &World,
    change_ids: &HashSet<ChangeId>,
) -> HashSet<LocusId> {
    change_ids
        .iter()
        .filter_map(|&change_id| world.log().get(change_id))
        .filter_map(|change| match change.subject {
            ChangeSubject::Locus(locus_id) => Some(locus_id),
            ChangeSubject::Relationship(_) => None,
        })
        .collect()
}

pub(super) fn collect_upstream_entity_transitions(
    world: &World,
    entity_id: EntityId,
    at_batch: BatchId,
    ancestor_loci: &HashSet<LocusId>,
) -> Vec<(EntityId, BatchId)> {
    let mut result = Vec::new();
    let mut seen_entities = HashSet::from([entity_id]);

    for entity in world.entities().iter() {
        if seen_entities.contains(&entity.id) {
            continue;
        }

        let Some(best_batch) = most_recent_overlapping_layer_batch(entity, at_batch, ancestor_loci)
        else {
            continue;
        };

        if world
            .entities()
            .layer_at_batch(entity.id, best_batch)
            .is_some()
        {
            seen_entities.insert(entity.id);
            result.push((entity.id, best_batch));
            if result.len() >= 256 {
                break;
            }
        }
    }

    result
}
fn most_recent_overlapping_layer_batch(
    entity: &graph_core::Entity,
    at_batch: BatchId,
    ancestor_loci: &HashSet<LocusId>,
) -> Option<BatchId> {
    entity
        .layers
        .iter()
        .rev()
        .filter(|layer| layer.batch < at_batch)
        .find_map(|layer| {
            let snapshot = layer.snapshot.as_ref()?;
            snapshot
                .members
                .iter()
                .any(|member| ancestor_loci.contains(member))
                .then_some(layer.batch)
        })
}
