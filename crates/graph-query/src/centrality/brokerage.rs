use graph_core::LocusId;
use graph_world::World;
use rustc_hash::{FxHashMap, FxHashSet};

pub fn structural_constraint(world: &World, locus: LocusId) -> Option<f32> {
    let neighbor_strengths = neighbor_strengths(world, locus);
    let total_strength: f32 = neighbor_strengths.values().sum();
    if total_strength == 0.0 || neighbor_strengths.is_empty() {
        return None;
    }

    let investment = direct_investment(&neighbor_strengths, total_strength);
    let neighborhood_strengths = neighborhood_strengths(world, &investment);
    let constraint = investment
        .iter()
        .map(|(&neighbor, &direct_share)| {
            let indirect_share =
                indirect_investment(&investment, &neighborhood_strengths, neighbor);
            (direct_share + indirect_share).powi(2)
        })
        .sum();

    Some(constraint)
}

pub fn all_constraints(world: &World) -> Vec<(LocusId, f32)> {
    let mut result: Vec<(LocusId, f32)> = world
        .loci()
        .iter()
        .filter_map(|locus| structural_constraint(world, locus.id).map(|score| (locus.id, score)))
        .collect();
    result.sort_by(|a, b| a.1.total_cmp(&b.1));
    result
}

pub fn effective_network_size(world: &World, locus: LocusId) -> f32 {
    let neighbor_strengths = neighbor_strengths(world, locus);
    let degree = neighbor_strengths.len() as f32;
    let total_strength: f32 = neighbor_strengths.values().sum();
    if total_strength == 0.0 || neighbor_strengths.is_empty() {
        return 0.0;
    }

    let locus_neighbors: FxHashSet<LocusId> = neighbor_strengths.keys().copied().collect();
    let redundancy: f32 = neighbor_strengths
        .iter()
        .map(|(&neighbor, &strength)| {
            let direct_share = strength / total_strength;
            direct_share * redundant_neighbor_share(world, locus, neighbor, &locus_neighbors)
        })
        .sum();

    degree - redundancy
}

fn neighbor_strengths(world: &World, locus: LocusId) -> FxHashMap<LocusId, f32> {
    let mut strengths = FxHashMap::default();
    for relationship in world.relationships_for_locus(locus) {
        let neighbor = relationship.endpoints.other_than(locus);
        *strengths.entry(neighbor).or_insert(0.0) += relationship.strength();
    }
    strengths
}

fn direct_investment(
    neighbor_strengths: &FxHashMap<LocusId, f32>,
    total_strength: f32,
) -> FxHashMap<LocusId, f32> {
    neighbor_strengths
        .iter()
        .map(|(&neighbor, &strength)| (neighbor, strength / total_strength))
        .collect()
}

fn neighborhood_strengths(
    world: &World,
    investment: &FxHashMap<LocusId, f32>,
) -> FxHashMap<LocusId, (FxHashMap<LocusId, f32>, f32)> {
    investment
        .keys()
        .map(|&neighbor| {
            let strengths = neighbor_strengths(world, neighbor);
            let total_strength: f32 = strengths.values().sum();
            (neighbor, (strengths, total_strength))
        })
        .collect()
}

fn indirect_investment(
    investment: &FxHashMap<LocusId, f32>,
    neighborhood_strengths: &FxHashMap<LocusId, (FxHashMap<LocusId, f32>, f32)>,
    target_neighbor: LocusId,
) -> f32 {
    investment
        .iter()
        .filter(|&(&neighbor, _)| neighbor != target_neighbor)
        .map(|(&neighbor, &direct_share)| {
            let neighbor_share = neighborhood_strengths
                .get(&neighbor)
                .map(|(strengths, total_strength)| {
                    if *total_strength > 0.0 {
                        strengths.get(&target_neighbor).copied().unwrap_or(0.0) / total_strength
                    } else {
                        0.0
                    }
                })
                .unwrap_or(0.0);
            direct_share * neighbor_share
        })
        .sum()
}

fn redundant_neighbor_share(
    world: &World,
    locus: LocusId,
    neighbor: LocusId,
    locus_neighbors: &FxHashSet<LocusId>,
) -> f32 {
    let neighbor_strengths = neighbor_strengths(world, neighbor);
    let total_strength: f32 = neighbor_strengths.values().sum();
    if total_strength == 0.0 {
        return 0.0;
    }

    neighbor_strengths
        .iter()
        .filter(|&(&other, _)| {
            other != neighbor && other != locus && locus_neighbors.contains(&other)
        })
        .map(|(_, &strength)| strength / total_strength)
        .sum()
}
