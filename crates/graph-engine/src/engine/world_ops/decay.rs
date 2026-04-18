use graph_core::{BatchId, InfluenceKindId, Relationship, RelationshipId, StateVector, WorldEvent};
use graph_world::World;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::registry::{DemotionPolicy, InfluenceKindRegistry};

struct RelationshipDecayPlan {
    rel_id: RelationshipId,
    after: StateVector,
    last_decayed_batch: u64,
}

pub(crate) fn flush_relationship_decay(
    world: &mut World,
    influence_registry: &InfluenceKindRegistry,
) -> (usize, Vec<WorldEvent>) {
    let current_batch = world.current_batch().0;
    let mut slot_cache: FxHashMap<InfluenceKindId, Vec<graph_core::RelationshipSlotDef>> =
        FxHashMap::default();

    apply_pending_relationship_decay(world, influence_registry, current_batch, &mut slot_cache);

    let to_prune = collect_decay_pruned_relationships();
    let events = build_pruned_relationship_events(&to_prune);
    remove_pruned_relationships(world, &to_prune);
    (to_prune.len(), events)
}

pub(crate) fn apply_demotion_policies(
    world: &mut World,
    influence_registry: &InfluenceKindRegistry,
    current_batch: BatchId,
) -> Vec<RelationshipId> {
    let mut to_evict: FxHashSet<RelationshipId> = FxHashSet::default();

    for kind in influence_registry.kinds() {
        let Some(policy) = influence_registry
            .get(kind)
            .and_then(|cfg| cfg.demotion_policy)
        else {
            continue;
        };
        to_evict.extend(collect_demoted_relationships(
            world,
            kind,
            policy,
            current_batch,
        ));
    }

    let evicted: Vec<RelationshipId> = to_evict.into_iter().collect();
    evict_relationships(world, &evicted);
    evicted
}

fn apply_pending_relationship_decay(
    world: &mut World,
    influence_registry: &InfluenceKindRegistry,
    current_batch: u64,
    slot_cache: &mut FxHashMap<InfluenceKindId, Vec<graph_core::RelationshipSlotDef>>,
) {
    let plans =
        build_relationship_decay_plans(world, influence_registry, current_batch, slot_cache);
    apply_relationship_decay_plans(world, &plans);
}

fn build_relationship_decay_plans(
    world: &World,
    influence_registry: &InfluenceKindRegistry,
    current_batch: u64,
    slot_cache: &mut FxHashMap<InfluenceKindId, Vec<graph_core::RelationshipSlotDef>>,
) -> Vec<RelationshipDecayPlan> {
    world
        .relationships()
        .iter()
        .filter_map(|rel| {
            build_relationship_decay_plan(rel, influence_registry, current_batch, slot_cache)
        })
        .collect()
}

fn build_relationship_decay_plan(
    rel: &Relationship,
    influence_registry: &InfluenceKindRegistry,
    current_batch: u64,
    slot_cache: &mut FxHashMap<InfluenceKindId, Vec<graph_core::RelationshipSlotDef>>,
) -> Option<RelationshipDecayPlan> {
    let delta = current_batch.saturating_sub(rel.last_decayed_batch);
    let cfg = influence_registry.get(rel.kind);
    debug_assert!(
        cfg.is_some(),
        "flush_relationship_decay: InfluenceKindId {:?} is not registered — relationship {:?} will not be decayed or pruned. Register it with InfluenceKindRegistry::insert().",
        rel.kind,
        rel.id
    );
    if delta == 0 {
        return None;
    }

    let (act_decay, wt_decay) = cfg
        .map(|c| (c.decay_per_batch, c.plasticity.weight_decay))
        .unwrap_or((1.0, 1.0));
    let act_factor = act_decay.powi(delta as i32);
    let wt_factor = wt_decay.powi(delta as i32);
    let mut after = rel.state.clone();
    let slots = after.as_mut_slice();
    if let Some(a) = slots.get_mut(graph_core::Relationship::ACTIVITY_SLOT) {
        *a *= act_factor;
    }
    if let Some(w) = slots.get_mut(graph_core::Relationship::WEIGHT_SLOT) {
        *w *= wt_factor;
    }

    let resolved_slots = slot_cache
        .entry(rel.kind)
        .or_insert_with(|| influence_registry.resolved_extra_slots(rel.kind));
    apply_extra_slot_decay(slots, resolved_slots, delta);
    Some(RelationshipDecayPlan {
        rel_id: rel.id,
        after,
        last_decayed_batch: current_batch,
    })
}

fn apply_relationship_decay_plans(world: &mut World, plans: &[RelationshipDecayPlan]) {
    for plan in plans {
        apply_relationship_decay_plan(world, plan);
    }
}

fn apply_relationship_decay_plan(world: &mut World, plan: &RelationshipDecayPlan) {
    if let Some(rel) = world.relationships_mut().get_mut(plan.rel_id) {
        rel.state = plan.after.clone();
        rel.last_decayed_batch = plan.last_decayed_batch;
    }
}

fn apply_extra_slot_decay(
    slots: &mut [f32],
    resolved_slots: &[graph_core::RelationshipSlotDef],
    delta: u64,
) {
    for (i, slot_def) in resolved_slots.iter().enumerate() {
        if let Some(factor) = slot_def.decay {
            let idx = 2 + i;
            if let Some(v) = slots.get_mut(idx) {
                *v *= factor.powi(delta as i32);
            }
        }
    }
}

fn collect_decay_pruned_relationships() -> Vec<RelationshipId> {
    Vec::new()
}

fn build_pruned_relationship_events(to_prune: &[RelationshipId]) -> Vec<WorldEvent> {
    to_prune
        .iter()
        .map(|&id| WorldEvent::RelationshipPruned { relationship: id })
        .collect()
}

fn remove_pruned_relationships(world: &mut World, to_prune: &[RelationshipId]) {
    for &rel_id in to_prune {
        world.subscriptions_mut().remove_relationship(rel_id);
        world.relationships_mut().remove(rel_id);
        world.record_pruned(rel_id);
    }
}

fn collect_demoted_relationships(
    world: &World,
    kind: InfluenceKindId,
    policy: DemotionPolicy,
    current_batch: BatchId,
) -> Vec<RelationshipId> {
    match policy {
        DemotionPolicy::ActivityFloor(floor) => world
            .relationships()
            .iter()
            .filter(|rel| rel.kind == kind && rel.activity() < floor)
            .map(|rel| rel.id)
            .collect(),
        DemotionPolicy::IdleBatches(n) => world
            .relationships()
            .iter()
            .filter(|rel| {
                rel.kind == kind && current_batch.0.saturating_sub(rel.last_decayed_batch) > n
            })
            .map(|rel| rel.id)
            .collect(),
        DemotionPolicy::LruCapacity(capacity) => collect_lru_demotions(world, kind, capacity),
    }
}

fn collect_lru_demotions(
    world: &World,
    kind: InfluenceKindId,
    capacity: usize,
) -> Vec<RelationshipId> {
    let mut rels_of_kind: Vec<(u64, RelationshipId)> = world
        .relationships()
        .iter()
        .filter(|rel| rel.kind == kind)
        .map(|rel| (rel.last_decayed_batch, rel.id))
        .collect();
    if rels_of_kind.len() <= capacity {
        return Vec::new();
    }

    rels_of_kind.sort_unstable_by(|a, b| b.0.cmp(&a.0));
    rels_of_kind[capacity..].iter().map(|(_, id)| *id).collect()
}

fn evict_relationships(world: &mut World, evicted: &[RelationshipId]) {
    for &id in evicted {
        world.relationships_mut().remove(id);
    }
}
