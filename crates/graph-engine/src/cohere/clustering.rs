use std::collections::VecDeque;

use graph_core::{Cohere, CohereMembers, Endpoints, EntityId, LocusId};
use graph_world::{EntityStore, RelationshipStore};
use rustc_hash::{FxHashMap, FxHashSet};

use super::DefaultCoherePerspective;

pub(super) fn cluster_default(
    perspective: &DefaultCoherePerspective,
    entities: &EntityStore,
    relationships: &RelationshipStore,
    next_id: &mut dyn FnMut() -> graph_core::CohereId,
) -> Vec<Cohere> {
    let active = active_entity_ids(entities);
    if active.is_empty() {
        return Vec::new();
    }

    let locus_to_entity = build_locus_entity_index(entities, &active);
    let pair_activity = accumulate_pair_activity(relationships, &locus_to_entity);
    let threshold = perspective
        .min_bridge_activity
        .unwrap_or_else(|| auto_bridge_threshold(&pair_activity));
    let bridges = build_bridge_adjacency(&pair_activity, threshold);

    collect_coheres(&active, &bridges, next_id)
}

fn active_entity_ids(entities: &EntityStore) -> Vec<EntityId> {
    entities.active().map(|e| e.id).collect()
}

fn build_locus_entity_index(
    entities: &EntityStore,
    active: &[EntityId],
) -> FxHashMap<LocusId, EntityId> {
    let mut locus_to_entity: FxHashMap<LocusId, EntityId> = FxHashMap::default();
    for &eid in active {
        if let Some(e) = entities.get(eid) {
            for &locus in &e.current.members {
                locus_to_entity.insert(locus, eid);
            }
        }
    }
    locus_to_entity
}

fn accumulate_pair_activity(
    relationships: &RelationshipStore,
    locus_to_entity: &FxHashMap<LocusId, EntityId>,
) -> FxHashMap<(EntityId, EntityId), f32> {
    let mut pair_activity: FxHashMap<(EntityId, EntityId), f32> = FxHashMap::default();
    for rel in relationships.iter() {
        let (from, to) = match &rel.endpoints {
            Endpoints::Directed { from, to } => (*from, *to),
            Endpoints::Symmetric { a, b } => (*a, *b),
        };
        let Some(&ea) = locus_to_entity.get(&from) else {
            continue;
        };
        let Some(&eb) = locus_to_entity.get(&to) else {
            continue;
        };
        if ea == eb {
            continue;
        }
        let key = if ea < eb { (ea, eb) } else { (eb, ea) };
        *pair_activity.entry(key).or_default() += rel.activity();
    }
    pair_activity
}

fn auto_bridge_threshold(pair_activity: &FxHashMap<(EntityId, EntityId), f32>) -> f32 {
    let mut values: Vec<f32> = pair_activity
        .values()
        .copied()
        .filter(|&a| a > 0.0)
        .collect();
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    values[values.len() / 2]
}

fn build_bridge_adjacency(
    pair_activity: &FxHashMap<(EntityId, EntityId), f32>,
    threshold: f32,
) -> FxHashMap<EntityId, Vec<EntityId>> {
    let mut bridges: FxHashMap<EntityId, Vec<EntityId>> = FxHashMap::default();
    for ((ea, eb), activity) in pair_activity {
        if *activity >= threshold {
            bridges.entry(*ea).or_default().push(*eb);
            bridges.entry(*eb).or_default().push(*ea);
        }
    }
    bridges
}

fn collect_coheres(
    active: &[EntityId],
    bridges: &FxHashMap<EntityId, Vec<EntityId>>,
    next_id: &mut dyn FnMut() -> graph_core::CohereId,
) -> Vec<Cohere> {
    let mut visited: FxHashSet<EntityId> = FxHashSet::default();
    let mut coheres = Vec::new();

    for &start in active {
        if visited.contains(&start) {
            continue;
        }
        let component = bfs_component(start, bridges, &mut visited);
        if component.len() >= 2 {
            coheres.push(build_cohere(component, bridges, next_id));
        }
    }

    coheres
}

fn bfs_component(
    start: EntityId,
    bridges: &FxHashMap<EntityId, Vec<EntityId>>,
    visited: &mut FxHashSet<EntityId>,
) -> Vec<EntityId> {
    let mut component = Vec::new();
    let mut queue = VecDeque::new();
    queue.push_back(start);
    visited.insert(start);
    while let Some(node) = queue.pop_front() {
        component.push(node);
        if let Some(neighbors) = bridges.get(&node) {
            for &nb in neighbors {
                if visited.insert(nb) {
                    queue.push_back(nb);
                }
            }
        }
    }
    component
}

fn build_cohere(
    mut component: Vec<EntityId>,
    bridges: &FxHashMap<EntityId, Vec<EntityId>>,
    next_id: &mut dyn FnMut() -> graph_core::CohereId,
) -> Cohere {
    let pair_count = component.len() * (component.len() - 1) / 2;
    let total_activity: f32 = bridges.values().flatten().count() as f32 / 2.0;
    let strength = if pair_count > 0 {
        (total_activity / pair_count as f32).min(1.0)
    } else {
        0.0
    };
    component.sort();
    Cohere {
        id: next_id(),
        members: CohereMembers::Entities(component),
        strength,
    }
}
