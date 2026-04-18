use graph_core::{BatchId, LocusId, RelationshipId};
use graph_world::World;

pub(super) fn locus_degree_inner(world: &World, locus: LocusId) -> usize {
    world.degree(locus)
}

pub(super) fn locus_in_degree_inner(world: &World, locus: LocusId) -> usize {
    world.in_degree(locus)
}

pub(super) fn locus_out_degree_inner(world: &World, locus: LocusId) -> usize {
    world.out_degree(locus)
}

pub(super) fn most_connected_loci_with_degree_inner(
    world: &World,
    n: usize,
) -> Vec<(LocusId, usize)> {
    if n == 0 {
        return Vec::new();
    }
    let mut by_degree: Vec<(LocusId, usize)> = world.degree_iter().collect();
    by_degree.sort_unstable_by_key(|&(_, degree)| std::cmp::Reverse(degree));
    by_degree.truncate(n);
    by_degree
}

pub(super) fn incoming_activity_sum_inner(world: &World, locus: LocusId) -> f32 {
    world
        .relationships_to(locus)
        .map(|relationship| relationship.activity())
        .sum()
}

pub(super) fn outgoing_activity_sum_inner(world: &World, locus: LocusId) -> f32 {
    world
        .relationships_from(locus)
        .map(|relationship| relationship.activity())
        .sum()
}

pub(super) fn net_influence_balance_inner(world: &World, locus: LocusId) -> f32 {
    outgoing_activity_sum_inner(world, locus) - incoming_activity_sum_inner(world, locus)
}

pub(super) fn relationship_touch_rate_inner(
    world: &World,
    rel_id: RelationshipId,
    current_batch: BatchId,
) -> f32 {
    let Some(rel) = world.relationships().get(rel_id) else {
        return 0.0;
    };
    let age = rel.age_in_batches(current_batch);
    if age == 0 {
        return 0.0;
    }
    rel.lineage.change_count as f32 / age as f32
}

pub(super) fn kind_flow_diversity_inner(world: &World, rel_id: RelationshipId) -> f32 {
    let Some(rel) = world.relationships().get(rel_id) else {
        return 0.0;
    };
    let lineage = &rel.lineage;
    if lineage.change_count == 0 {
        return 0.0;
    }
    lineage.kinds_observed.len() as f32 / lineage.change_count as f32
}

pub(super) fn kind_transition_rate_inner(world: &World, rel_id: RelationshipId) -> f32 {
    let Some(rel) = world.relationships().get(rel_id) else {
        return 0.0;
    };
    let lineage = &rel.lineage;
    if lineage.kinds_observed.is_empty() {
        return 0.0;
    }
    let latest_batch = lineage
        .kinds_observed
        .iter()
        .map(|observation| observation.last_batch.0)
        .max()
        .unwrap_or(rel.created_batch.0);
    let age = latest_batch.saturating_sub(rel.created_batch.0);
    if age == 0 {
        return 0.0;
    }
    lineage.kinds_observed.len() as f32 / age as f32
}
