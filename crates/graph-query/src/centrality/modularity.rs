use graph_core::LocusId;
use graph_world::World;
use rustc_hash::FxHashMap;

pub fn modularity(world: &World, partition: &[Vec<LocusId>]) -> f32 {
    if partition.is_empty() {
        return 0.0;
    }

    let group_of = group_membership(partition);
    let strengths = node_strengths(world);
    let total_strength: f32 = strengths.values().sum();
    if total_strength == 0.0 {
        return 0.0;
    }

    let within_group_strength = within_group_strength(world, &group_of);
    let null_model_strength = null_model_strength(partition, &strengths) / total_strength;
    (within_group_strength - null_model_strength) / total_strength
}

fn group_membership(partition: &[Vec<LocusId>]) -> FxHashMap<LocusId, usize> {
    let mut group_of = FxHashMap::default();
    for (group, members) in partition.iter().enumerate() {
        for &id in members {
            group_of.insert(id, group);
        }
    }
    group_of
}

fn node_strengths(world: &World) -> FxHashMap<LocusId, f32> {
    let mut strengths = FxHashMap::default();
    for relationship in world.relationships().iter() {
        let (from, to) = endpoints(&relationship.endpoints);
        let strength = relationship.strength();
        *strengths.entry(from).or_insert(0.0) += strength;
        *strengths.entry(to).or_insert(0.0) += strength;
    }
    strengths
}

fn within_group_strength(world: &World, group_of: &FxHashMap<LocusId, usize>) -> f32 {
    let mut total = 0.0;
    for relationship in world.relationships().iter() {
        let (from, to) = endpoints(&relationship.endpoints);
        if let (Some(&from_group), Some(&to_group)) = (group_of.get(&from), group_of.get(&to))
            && from_group == to_group
        {
            total += 2.0 * relationship.strength();
        }
    }
    total
}

fn null_model_strength(partition: &[Vec<LocusId>], strengths: &FxHashMap<LocusId, f32>) -> f32 {
    partition
        .iter()
        .map(|members| {
            let group_strength: f32 = members
                .iter()
                .map(|id| strengths.get(id).copied().unwrap_or(0.0))
                .sum();
            group_strength * group_strength
        })
        .sum()
}

fn endpoints(endpoints: &graph_core::Endpoints) -> (LocusId, LocusId) {
    match endpoints {
        graph_core::Endpoints::Symmetric { a, b } => (*a, *b),
        graph_core::Endpoints::Directed { from, to } => (*from, *to),
    }
}
