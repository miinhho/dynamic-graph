use graph_core::LocusId;
use graph_world::World;
use rustc_hash::{FxHashMap, FxHashSet};

pub(super) struct IndexedLoci {
    pub(super) loci: Vec<LocusId>,
    pub(super) idx: FxHashMap<LocusId, usize>,
}

pub(super) fn index_loci(world: &World) -> IndexedLoci {
    let loci: Vec<LocusId> = world.loci().iter().map(|l| l.id).collect();
    let idx = loci.iter().enumerate().map(|(i, &id)| (id, i)).collect();
    IndexedLoci { loci, idx }
}

pub(super) fn unweighted_undirected_adj(
    world: &World,
    idx: &FxHashMap<LocusId, usize>,
    n: usize,
) -> Vec<Vec<usize>> {
    let mut adj_set: Vec<FxHashSet<usize>> = vec![FxHashSet::default(); n];
    for rel in world.relationships().iter() {
        let (u, v) = endpoints(rel);
        if let (Some(&ui), Some(&vi)) = (idx.get(&u), idx.get(&v)) {
            adj_set[ui].insert(vi);
            adj_set[vi].insert(ui);
        }
    }
    adj_set
        .into_iter()
        .map(|neighbors| {
            let mut neighbors: Vec<usize> = neighbors.into_iter().collect();
            neighbors.sort_unstable();
            neighbors
        })
        .collect()
}

pub(super) fn weighted_undirected_adj(
    world: &World,
    idx: &FxHashMap<LocusId, usize>,
    n: usize,
) -> Vec<Vec<(usize, f32)>> {
    let mut adj_map: Vec<FxHashMap<usize, f32>> = vec![FxHashMap::default(); n];
    for rel in world.relationships().iter() {
        let weight = rel.activity().max(0.0);
        if weight == 0.0 {
            continue;
        }
        let (u, v) = endpoints(rel);
        if let (Some(&ui), Some(&vi)) = (idx.get(&u), idx.get(&v)) {
            *adj_map[ui].entry(vi).or_insert(0.0) += weight;
            *adj_map[vi].entry(ui).or_insert(0.0) += weight;
        }
    }
    adj_map
        .into_iter()
        .map(|edges| edges.into_iter().collect())
        .collect()
}

pub(super) fn weighted_directed_in_edges(
    world: &World,
    idx: &FxHashMap<LocusId, usize>,
    n: usize,
) -> (Vec<Vec<(usize, f32)>>, Vec<f32>) {
    let mut in_edges: Vec<Vec<(usize, f32)>> = vec![Vec::new(); n];
    let mut out_activity: Vec<f32> = vec![0.0; n];

    for rel in world.relationships().iter() {
        let activity = rel.activity().max(0.0);
        if activity == 0.0 {
            continue;
        }
        match rel.endpoints {
            graph_core::Endpoints::Directed { from, to } => {
                if let (Some(&ui), Some(&vi)) = (idx.get(&from), idx.get(&to)) {
                    in_edges[vi].push((ui, activity));
                    out_activity[ui] += activity;
                }
            }
            graph_core::Endpoints::Symmetric { a, b } => {
                if let (Some(&ai), Some(&bi)) = (idx.get(&a), idx.get(&b)) {
                    in_edges[bi].push((ai, activity));
                    in_edges[ai].push((bi, activity));
                    out_activity[ai] += activity;
                    out_activity[bi] += activity;
                }
            }
        }
    }

    for edges in &mut in_edges {
        for (source, weight) in edges {
            let total = out_activity[*source];
            if total > 0.0 {
                *weight /= total;
            }
        }
    }

    (in_edges, out_activity)
}

pub(super) fn collect_sorted_communities(
    loci: &[LocusId],
    community: &[usize],
) -> Vec<Vec<LocusId>> {
    let mut groups: FxHashMap<usize, Vec<LocusId>> = FxHashMap::default();
    for (i, &id) in loci.iter().enumerate() {
        groups.entry(community[i]).or_default().push(id);
    }
    let mut result: Vec<Vec<LocusId>> = groups.into_values().collect();
    for group in &mut result {
        group.sort();
    }
    result.sort_by(|a, b| b.len().cmp(&a.len()).then(a[0].cmp(&b[0])));
    result
}

fn endpoints(rel: &graph_core::Relationship) -> (LocusId, LocusId) {
    match rel.endpoints {
        graph_core::Endpoints::Symmetric { a, b } => (a, b),
        graph_core::Endpoints::Directed { from, to } => (from, to),
    }
}
