use graph_core::{LocusId, Relationship, RelationshipId};
use graph_world::RelationshipStore;
use rustc_hash::FxHashMap;

use super::{AdjMap, CommunityResult, LocalAdjEntry, LocalCommunityGraph};

pub(super) fn find_communities(store: &RelationshipStore, threshold: f32) -> CommunityResult {
    let adj = build_active_adjacency(store, threshold);
    if adj.is_empty() {
        return CommunityResult {
            components: Vec::new(),
            adj,
        };
    }

    let local_graph = build_local_community_graph(&adj);
    let labels = propagate_labels(&local_graph.local_adj);
    let components = collect_components(&local_graph.all_loci, &labels);

    CommunityResult { components, adj }
}

pub(super) fn component_stats(
    member_set: &rustc_hash::FxHashSet<LocusId>,
    adj: &AdjMap,
    threshold: f32,
) -> (f32, Vec<RelationshipId>) {
    let mut sum = 0.0f32;
    let mut active_count = 0usize;
    let mut rel_ids = Vec::new();
    for &locus in member_set {
        if let Some(neighbors) = adj.get(&locus) {
            for &(nb, rel_id, activity) in neighbors {
                if nb > locus && member_set.contains(&nb) {
                    rel_ids.push(rel_id);
                    if activity >= threshold {
                        sum += activity;
                        active_count += 1;
                    }
                }
            }
        }
    }
    let mean_activity = if active_count == 0 {
        0.0
    } else {
        sum / active_count as f32
    };
    let n = member_set.len();
    let reference = if n <= 1 {
        1.0f32
    } else {
        (n as f32) * ((n as f32 + 1.0).ln()) / 2.0
    };
    let density = (active_count as f32 / reference).min(1.0);
    (mean_activity * density, rel_ids)
}

fn build_active_adjacency(store: &RelationshipStore, threshold: f32) -> AdjMap {
    let mut adj: AdjMap = FxHashMap::default();
    for rel in store.iter() {
        if rel.activity().abs() < threshold {
            continue;
        }
        let (a, b) = endpoints_pair(rel);
        let w = rel.activity() + rel.weight();
        adj.entry(a).or_default().push((b, rel.id, w));
        adj.entry(b).or_default().push((a, rel.id, w));
    }
    adj
}

fn build_local_community_graph(adj: &AdjMap) -> LocalCommunityGraph {
    let mut all_loci: Vec<LocusId> = adj.keys().copied().collect();
    all_loci.sort();

    let mut locus_to_idx: FxHashMap<LocusId, usize> =
        FxHashMap::with_capacity_and_hasher(all_loci.len(), Default::default());
    for (idx, &locus) in all_loci.iter().enumerate() {
        locus_to_idx.insert(locus, idx);
    }

    let local_adj = all_loci
        .iter()
        .map(|locus| {
            adj[locus]
                .iter()
                .map(|&(neighbor, rel_id, weight)| (locus_to_idx[&neighbor], rel_id, weight))
                .collect()
        })
        .collect();

    LocalCommunityGraph {
        all_loci,
        local_adj,
    }
}

fn propagate_labels(local_adj: &[Vec<LocalAdjEntry>]) -> Vec<usize> {
    let node_count = local_adj.len();
    let mut labels: Vec<usize> = (0..node_count).collect();
    let mut label_weight: Vec<f32> = vec![0.0; node_count];
    let mut seen: Vec<bool> = vec![false; node_count];
    let mut dirty_labels: Vec<usize> = Vec::new();
    let mut order: Vec<usize> = (0..node_count).collect();
    let mut lcg_state: u64 = 0x517cc1b727220a95;

    for _ in 0..15 {
        shuffle_visit_order(&mut order, &mut lcg_state);

        let mut changed = false;
        for &node in &order {
            let best_label = choose_node_label(
                node,
                local_adj,
                &labels,
                &mut label_weight,
                &mut seen,
                &mut dirty_labels,
            );
            if labels[node] != best_label {
                labels[node] = best_label;
                changed = true;
            }
        }

        if !changed {
            break;
        }
    }

    labels
}

fn shuffle_visit_order(order: &mut [usize], lcg_state: &mut u64) {
    for i in (1..order.len()).rev() {
        *lcg_state = lcg_state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let j = (*lcg_state >> 33) as usize % (i + 1);
        order.swap(i, j);
    }
}

fn choose_node_label(
    node: usize,
    local_adj: &[Vec<LocalAdjEntry>],
    labels: &[usize],
    label_weight: &mut [f32],
    seen: &mut [bool],
    dirty_labels: &mut Vec<usize>,
) -> usize {
    dirty_labels.clear();

    for &(neighbor, _, weight) in &local_adj[node] {
        let label = labels[neighbor];
        if !seen[label] {
            seen[label] = true;
            dirty_labels.push(label);
        }
        label_weight[label] += weight;
    }

    let best_label = dirty_labels
        .iter()
        .copied()
        .max_by(|&a, &b| {
            label_weight[a]
                .partial_cmp(&label_weight[b])
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(b.cmp(&a))
        })
        .unwrap_or(labels[node]);

    for &label in dirty_labels.iter() {
        label_weight[label] = 0.0;
        seen[label] = false;
    }

    best_label
}

fn collect_components(all_loci: &[LocusId], labels: &[usize]) -> Vec<Vec<LocusId>> {
    let mut groups: FxHashMap<usize, Vec<LocusId>> = FxHashMap::default();
    for (idx, &label) in labels.iter().enumerate() {
        groups.entry(label).or_default().push(all_loci[idx]);
    }

    let mut components: Vec<Vec<LocusId>> = groups.into_values().collect();
    for component in &mut components {
        component.sort();
    }
    components.sort_by(|a, b| a[0].0.cmp(&b[0].0));
    components
}

fn endpoints_pair(rel: &Relationship) -> (LocusId, LocusId) {
    use graph_core::Endpoints;
    match &rel.endpoints {
        Endpoints::Directed { from, to } => (*from, *to),
        Endpoints::Symmetric { a, b } => (*a, *b),
    }
}
