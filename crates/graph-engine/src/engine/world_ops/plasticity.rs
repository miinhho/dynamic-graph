use graph_core::{
    EndpointKey, InfluenceKindId, InteractionEffect, Relationship, RelationshipId, StateVector,
};
use graph_world::World;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::engine::batch::PlasticityObs;
use crate::registry::InfluenceKindRegistry;

pub(crate) struct HebbianEffect {
    pub(crate) rel_id: RelationshipId,
    pub(crate) kind: InfluenceKindId,
    pub(crate) before: StateVector,
    pub(crate) after: StateVector,
}

struct InteractionEffectPlan {
    multiplier: f32,
    rel_ids: Vec<RelationshipId>,
}

pub(crate) fn compute_hebbian_effects(
    world: &World,
    obs: &[PlasticityObs],
    influence_registry: &InfluenceKindRegistry,
) -> Vec<HebbianEffect> {
    obs.iter()
        .filter_map(|obs| compute_hebbian_effect(world, influence_registry, *obs))
        .collect()
}

pub(crate) fn apply_hebbian_effects(world: &mut World, effects: &[HebbianEffect]) {
    for effect in effects {
        apply_hebbian_effect(world, effect);
    }
}

pub(crate) fn apply_interaction_effects(
    world: &mut World,
    batch_kind_touches: &FxHashMap<
        EndpointKey,
        (FxHashSet<InfluenceKindId>, FxHashSet<RelationshipId>),
    >,
    influence_registry: &InfluenceKindRegistry,
) {
    for plan in build_interaction_effect_plans(batch_kind_touches, influence_registry) {
        apply_interaction_effect_plan(world, plan);
    }
}

fn compute_hebbian_effect(
    world: &World,
    influence_registry: &InfluenceKindRegistry,
    obs: PlasticityObs,
) -> Option<HebbianEffect> {
    let PlasticityObs {
        rel_id,
        kind,
        pre,
        post,
        timing: _,
        post_locus: _,
    } = obs;
    let (eta, max_w) = hebbian_params(influence_registry, kind)?;
    compute_hebbian_weight_update(world, rel_id, pre, post, eta, max_w).map(|(before, after)| {
        HebbianEffect {
            rel_id,
            kind,
            before,
            after,
        }
    })
}

fn hebbian_params(
    influence_registry: &InfluenceKindRegistry,
    kind: InfluenceKindId,
) -> Option<(f32, f32)> {
    influence_registry
        .get(kind)
        .map(|cfg| (cfg.plasticity.learning_rate, cfg.plasticity.max_weight))
}

fn compute_hebbian_weight_update(
    world: &World,
    rel_id: RelationshipId,
    pre: f32,
    post: f32,
    eta: f32,
    max_w: f32,
) -> Option<(StateVector, StateVector)> {
    let rel = world.relationships().get(rel_id)?;
    let cur_w = rel
        .state
        .as_slice()
        .get(Relationship::WEIGHT_SLOT)
        .copied()
        .unwrap_or(0.0);
    let new_w = compute_hebbian_weight(cur_w, pre, post, eta, max_w);
    if (new_w - cur_w).abs() <= 1e-9 {
        return None;
    }
    let before = rel.state.clone();
    let mut after = before.clone();
    after.as_mut_slice()[Relationship::WEIGHT_SLOT] = new_w;
    Some((before, after))
}

fn apply_hebbian_effect(world: &mut World, effect: &HebbianEffect) {
    if let Some(rel) = world.relationships_mut().get_mut(effect.rel_id) {
        rel.state = effect.after.clone();
    }
}

fn compute_hebbian_weight(cur_w: f32, pre: f32, post: f32, eta: f32, max_w: f32) -> f32 {
    (cur_w + eta * pre * post).clamp(0.0, max_w)
}

fn interaction_multiplier(
    touched_kinds: &FxHashSet<InfluenceKindId>,
    influence_registry: &InfluenceKindRegistry,
) -> f32 {
    if touched_kinds.len() < 2 {
        return 1.0;
    }
    let kinds: Vec<InfluenceKindId> = touched_kinds.iter().copied().collect();
    let mut multiplier = 1.0f32;
    for i in 0..kinds.len() {
        for j in (i + 1)..kinds.len() {
            if let Some(effect) = influence_registry.interaction_between(kinds[i], kinds[j]) {
                multiplier *= effect_multiplier(effect);
            }
        }
    }
    multiplier
}

fn build_interaction_effect_plans(
    batch_kind_touches: &FxHashMap<
        EndpointKey,
        (FxHashSet<InfluenceKindId>, FxHashSet<RelationshipId>),
    >,
    influence_registry: &InfluenceKindRegistry,
) -> Vec<InteractionEffectPlan> {
    batch_kind_touches
        .values()
        .filter_map(|(touched_kinds, rel_ids)| {
            let multiplier = interaction_multiplier(touched_kinds, influence_registry);
            ((multiplier - 1.0).abs() > f32::EPSILON).then(|| InteractionEffectPlan {
                multiplier,
                rel_ids: rel_ids.iter().copied().collect(),
            })
        })
        .collect()
}

fn effect_multiplier(effect: &InteractionEffect) -> f32 {
    match effect {
        InteractionEffect::Synergistic { boost } => *boost,
        InteractionEffect::Antagonistic { dampen } => *dampen,
        InteractionEffect::Neutral => 1.0,
    }
}

fn apply_interaction_effect_plan(world: &mut World, plan: InteractionEffectPlan) {
    for rel_id in plan.rel_ids {
        if let Some(rel) = world.relationships_mut().get_mut(rel_id)
            && let Some(activity) = rel
                .state
                .as_mut_slice()
                .get_mut(Relationship::ACTIVITY_SLOT)
        {
            *activity *= plan.multiplier;
        }
    }
}
