use graph_core::LocusId;
use graph_world::World;

use super::indexed::{
    IndexedLoci, collect_sorted_communities, index_loci, weighted_undirected_adj,
};

pub fn louvain(world: &World) -> Vec<Vec<LocusId>> {
    louvain_with_resolution(world, 1.0)
}

pub fn louvain_with_resolution(world: &World, gamma: f32) -> Vec<Vec<LocusId>> {
    let IndexedLoci { loci, idx } = index_loci(world);
    let node_count = loci.len();
    if node_count == 0 {
        return Vec::new();
    }
    let adjacency = weighted_undirected_adj(world, &idx, node_count);
    let degrees = weighted_degrees(&adjacency);
    let total_weight: f32 = degrees.iter().sum();

    if total_weight == 0.0 {
        return loci.into_iter().map(|id| vec![id]).collect();
    }

    let mut state = CommunityState::new(degrees);
    let mut neighbor_communities = Vec::new();

    while run_louvain_pass(
        &adjacency,
        gamma,
        total_weight,
        &mut state,
        &mut neighbor_communities,
    ) {}

    collect_sorted_communities(&loci, &state.community)
}

fn weighted_degrees(adjacency: &[Vec<(usize, f32)>]) -> Vec<f32> {
    adjacency
        .iter()
        .map(|edges| edges.iter().map(|(_, weight)| weight).sum())
        .collect()
}

fn run_louvain_pass(
    adjacency: &[Vec<(usize, f32)>],
    gamma: f32,
    total_weight: f32,
    state: &mut CommunityState,
    neighbor_communities: &mut Vec<(usize, f32)>,
) -> bool {
    let mut improved = false;

    for node in 0..adjacency.len() {
        let current_community = state.community[node];
        let score_stay = current_community_score(
            adjacency,
            node,
            current_community,
            gamma,
            total_weight,
            state,
        );
        collect_neighbor_communities(
            adjacency,
            node,
            current_community,
            &state.community,
            neighbor_communities,
        );
        let best_community = best_target_community(
            node,
            score_stay,
            gamma,
            total_weight,
            state,
            neighbor_communities,
        );

        if best_community != current_community {
            state.move_node(node, best_community);
            improved = true;
        }
    }

    improved
}

fn current_community_score(
    adjacency: &[Vec<(usize, f32)>],
    node: usize,
    community: usize,
    gamma: f32,
    total_weight: f32,
    state: &CommunityState,
) -> f32 {
    let weight_in_community: f32 = adjacency[node]
        .iter()
        .filter(|&&(other, _)| state.community[other] == community)
        .map(|(_, weight)| weight)
        .sum();
    weight_in_community
        - gamma * state.degree[node] * (state.sigma_tot[community] - state.degree[node])
            / total_weight
}

fn collect_neighbor_communities(
    adjacency: &[Vec<(usize, f32)>],
    node: usize,
    current_community: usize,
    community_of: &[usize],
    neighbor_communities: &mut Vec<(usize, f32)>,
) {
    neighbor_communities.clear();

    for &(neighbor, weight) in &adjacency[node] {
        let community = community_of[neighbor];
        if community == current_community {
            continue;
        }

        if let Some(entry) = neighbor_communities
            .iter_mut()
            .find(|(candidate, _)| *candidate == community)
        {
            entry.1 += weight;
        } else {
            neighbor_communities.push((community, weight));
        }
    }
}

fn best_target_community(
    node: usize,
    score_stay: f32,
    gamma: f32,
    total_weight: f32,
    state: &CommunityState,
    neighbor_communities: &[(usize, f32)],
) -> usize {
    let mut best_community = state.community[node];
    let mut best_score = score_stay;

    for &(community, weight_in_community) in neighbor_communities {
        let score = weight_in_community
            - gamma * state.degree[node] * state.sigma_tot[community] / total_weight;
        if score > best_score {
            best_score = score;
            best_community = community;
        }
    }

    best_community
}

struct CommunityState {
    community: Vec<usize>,
    degree: Vec<f32>,
    sigma_tot: Vec<f32>,
}

impl CommunityState {
    fn new(degree: Vec<f32>) -> Self {
        let community = (0..degree.len()).collect();
        let sigma_tot = degree.clone();
        Self {
            community,
            degree,
            sigma_tot,
        }
    }

    fn move_node(&mut self, node: usize, target_community: usize) {
        let current_community = self.community[node];
        self.sigma_tot[current_community] -= self.degree[node];
        self.sigma_tot[target_community] += self.degree[node];
        self.community[node] = target_community;
    }
}
