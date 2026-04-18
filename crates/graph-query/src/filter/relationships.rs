use super::metrics::{
    incoming_activity_sum_inner, kind_flow_diversity_inner, kind_transition_rate_inner,
    net_influence_balance_inner, outgoing_activity_sum_inner, relationship_touch_rate_inner,
};
use graph_core::{
    BatchId, InfluenceKindId, InteractionEffect, LocusId, Relationship, RelationshipId,
};
use graph_world::World;

pub type SlotCondition = (usize, Box<dyn Fn(f32) -> bool>);

pub fn relationships_of_kind(world: &World, kind: InfluenceKindId) -> Vec<&Relationship> {
    world
        .relationships()
        .iter()
        .filter(|r| r.kind == kind)
        .collect()
}

pub fn relationships_of_kinds<'w>(
    world: &'w World,
    kinds: &[InfluenceKindId],
) -> Vec<&'w Relationship> {
    if kinds.is_empty() {
        return Vec::new();
    }
    world
        .relationships()
        .iter()
        .filter(|r| kinds.contains(&r.kind))
        .collect()
}

pub fn relationships_with_activity<F>(world: &World, pred: F) -> Vec<&Relationship>
where
    F: Fn(f32) -> bool,
{
    world
        .relationships()
        .iter()
        .filter(|r| pred(r.activity()))
        .collect()
}

pub fn relationships_with_weight<F>(world: &World, pred: F) -> Vec<&Relationship>
where
    F: Fn(f32) -> bool,
{
    world
        .relationships()
        .iter()
        .filter(|r| pred(r.weight()))
        .collect()
}

pub fn relationships_with_slot<F>(world: &World, slot_idx: usize, pred: F) -> Vec<&Relationship>
where
    F: Fn(f32) -> bool,
{
    world
        .relationships()
        .iter()
        .filter(|r| r.state.as_slice().get(slot_idx).is_some_and(|&v| pred(v)))
        .collect()
}

pub fn most_similar_relationships(
    world: &World,
    rel_id: RelationshipId,
    n: usize,
) -> Vec<(RelationshipId, f32)> {
    if n == 0 {
        return Vec::new();
    }
    let anchor = match world.relationships().get(rel_id) {
        Some(r) => r,
        None => return Vec::new(),
    };
    let anchor_state = anchor.state.clone();

    let mut scored: Vec<(RelationshipId, f32)> = world
        .relationships()
        .iter()
        .filter(|r| r.id != rel_id)
        .map(|r| (r.id, anchor_state.cosine_similarity(&r.state)))
        .collect();

    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(n);
    scored
}

pub fn relationships_matching<F>(world: &World, pred: F) -> Vec<&Relationship>
where
    F: Fn(&Relationship) -> bool,
{
    world.relationships().iter().filter(|r| pred(r)).collect()
}

pub fn relationships_matching_slots(
    world: &World,
    conditions: Vec<SlotCondition>,
) -> Vec<&Relationship> {
    if conditions.is_empty() {
        return world.relationships().iter().collect();
    }
    world
        .relationships()
        .iter()
        .filter(|r| {
            conditions
                .iter()
                .all(|(slot_idx, pred)| r.state.as_slice().get(*slot_idx).is_some_and(|&v| pred(v)))
        })
        .collect()
}

pub fn relationships_with_str_property<'w, F>(
    world: &'w World,
    key: &str,
    pred: F,
) -> Vec<&'w Relationship>
where
    F: Fn(&str) -> bool,
{
    world
        .relationships()
        .iter()
        .filter(|r| r.get_str_property(key).is_some_and(&pred))
        .collect()
}

pub fn relationships_with_f64_property<'w, F>(
    world: &'w World,
    key: &str,
    pred: F,
) -> Vec<&'w Relationship>
where
    F: Fn(f64) -> bool,
{
    world
        .relationships()
        .iter()
        .filter(|r| r.get_f64_property(key).is_some_and(&pred))
        .collect()
}

pub fn relationships_created_in(world: &World, from: BatchId, to: BatchId) -> Vec<&Relationship> {
    world
        .relationships()
        .iter()
        .filter(|r| r.created_batch >= from && r.created_batch <= to)
        .collect()
}

pub fn relationships_older_than(
    world: &World,
    current_batch: BatchId,
    min_batches: u64,
) -> Vec<&Relationship> {
    world
        .relationships()
        .iter()
        .filter(|r| r.age_in_batches(current_batch) >= min_batches)
        .collect()
}

pub fn incoming_activity_sum(world: &World, locus: LocusId) -> f32 {
    incoming_activity_sum_inner(world, locus)
}

pub fn outgoing_activity_sum(world: &World, locus: LocusId) -> f32 {
    outgoing_activity_sum_inner(world, locus)
}

pub fn net_influence_balance(world: &World, locus: LocusId) -> f32 {
    net_influence_balance_inner(world, locus)
}

pub fn net_influence_between<F>(world: &World, a: LocusId, b: LocusId, interaction_fn: F) -> f32
where
    F: Fn(InfluenceKindId, InfluenceKindId) -> Option<InteractionEffect>,
{
    crate::relationship_profile(world, a, b).net_activity_with_interactions(interaction_fn)
}

pub fn relationships_by_change_count(world: &World, min_count: u64) -> Vec<&Relationship> {
    world
        .relationships()
        .iter()
        .filter(|r| r.lineage.change_count >= min_count)
        .collect()
}

pub fn most_changed_relationships(world: &World, n: usize) -> Vec<&Relationship> {
    if n == 0 {
        return Vec::new();
    }
    let mut all: Vec<&Relationship> = world.relationships().iter().collect();
    all.sort_unstable_by_key(|r| std::cmp::Reverse(r.lineage.change_count));
    all.truncate(n);
    all
}

pub fn relationships_above_strength(world: &World, threshold: f32) -> Vec<&Relationship> {
    world
        .relationships()
        .iter()
        .filter(|r| r.strength() > threshold)
        .collect()
}

pub fn relationships_top_n_by_strength(world: &World, n: usize) -> Vec<&Relationship> {
    if n == 0 {
        return Vec::new();
    }
    let mut all: Vec<&Relationship> = world.relationships().iter().collect();
    all.sort_unstable_by(|a, b| b.strength().total_cmp(&a.strength()));
    all.truncate(n);
    all
}

pub fn relationship_touch_rate(
    world: &World,
    rel_id: RelationshipId,
    current_batch: BatchId,
) -> f32 {
    relationship_touch_rate_inner(world, rel_id, current_batch)
}

pub fn relationships_idle_for(
    world: &World,
    current_batch: BatchId,
    min_idle_batches: u64,
) -> Vec<&Relationship> {
    world
        .relationships()
        .iter()
        .filter(|r| current_batch.0.saturating_sub(r.last_decayed_batch) >= min_idle_batches)
        .collect()
}

pub fn relationships_from(world: &World, locus: LocusId) -> Vec<&Relationship> {
    world.relationships_from(locus).collect()
}

pub fn relationships_from_of_kind(
    world: &World,
    locus: LocusId,
    kind: InfluenceKindId,
) -> Vec<&Relationship> {
    world.relationships_from_of_kind(locus, kind).collect()
}

pub fn relationships_to(world: &World, locus: LocusId) -> Vec<&Relationship> {
    world.relationships_to(locus).collect()
}

pub fn relationships_to_of_kind(
    world: &World,
    locus: LocusId,
    kind: InfluenceKindId,
) -> Vec<&Relationship> {
    world.relationships_to_of_kind(locus, kind).collect()
}

pub fn relationships_between(world: &World, a: LocusId, b: LocusId) -> Vec<&Relationship> {
    crate::relationship_profile(world, a, b).relationships
}

pub fn relationships_between_of_kind(
    world: &World,
    a: LocusId,
    b: LocusId,
    kind: InfluenceKindId,
) -> Vec<&Relationship> {
    world.relationships_between_of_kind(a, b, kind).collect()
}

pub fn dominant_flow_kind(world: &World, rel_id: RelationshipId) -> Option<InfluenceKindId> {
    world
        .relationships()
        .get(rel_id)?
        .lineage
        .dominant_flow_kind()
}

pub fn kind_flow_diversity(world: &World, rel_id: RelationshipId) -> f32 {
    kind_flow_diversity_inner(world, rel_id)
}

pub fn kind_transition_rate(world: &World, rel_id: RelationshipId) -> f32 {
    kind_transition_rate_inner(world, rel_id)
}
