use std::collections::VecDeque;

use graph_core::LocusId;
use graph_world::World;
use rustc_hash::FxHashMap;

use super::indexed::{
    IndexedLoci, index_loci, unweighted_undirected_adj, weighted_directed_in_edges,
};

pub(super) fn all_closeness(world: &World) -> Vec<(LocusId, f32)> {
    let IndexedLoci { loci, idx } = index_loci(world);
    let node_count = loci.len();
    if node_count < 2 {
        return Vec::new();
    }
    let adjacency = unweighted_undirected_adj(world, &idx, node_count);
    let mut state = ClosenessState::new(node_count);
    let denominator = node_count as f32 - 1.0;

    let mut result: Vec<(LocusId, f32)> = (0..node_count)
        .map(|source| {
            let harmonic = state.harmonic_sum(source, &adjacency);
            (loci[source], harmonic / denominator)
        })
        .collect();
    result.sort_by(|a, b| b.1.total_cmp(&a.1));
    result
}

pub(super) fn bfs_harmonic_sum(world: &World, start: LocusId) -> f32 {
    let mut dist: FxHashMap<LocusId, u32> = FxHashMap::default();
    let mut queue: VecDeque<LocusId> = VecDeque::new();
    dist.insert(start, 0);
    queue.push_back(start);
    let mut harmonic = 0.0;

    while let Some(current) = queue.pop_front() {
        let depth = dist[&current];
        for relationship in world.relationships_for_locus(current) {
            let neighbor = relationship.endpoints.other_than(current);
            if let std::collections::hash_map::Entry::Vacant(entry) = dist.entry(neighbor) {
                let next_depth = depth + 1;
                entry.insert(next_depth);
                harmonic += 1.0 / next_depth as f32;
                queue.push_back(neighbor);
            }
        }
    }

    harmonic
}

pub(super) fn pagerank(
    world: &World,
    damping: f32,
    max_iter: usize,
    tol: f32,
) -> Vec<(LocusId, f32)> {
    let IndexedLoci { loci, idx } = index_loci(world);
    let node_count = loci.len();
    if node_count == 0 {
        return Vec::new();
    }

    let (in_edges, out_activity) = weighted_directed_in_edges(world, &idx, node_count);
    let mut state = PageRankState::new(node_count, damping);
    let dangling = dangling_nodes(&out_activity);

    for _ in 0..max_iter {
        let delta = state.iterate(&in_edges, &dangling);
        if delta < tol {
            break;
        }
    }

    let mut result: Vec<(LocusId, f32)> = loci
        .into_iter()
        .enumerate()
        .map(|(index, id)| (id, state.rank[index]))
        .collect();
    result.sort_by(|a, b| b.1.total_cmp(&a.1));
    result
}

fn dangling_nodes(out_activity: &[f32]) -> Vec<usize> {
    (0..out_activity.len())
        .filter(|&index| out_activity[index] == 0.0)
        .collect()
}

struct ClosenessState {
    dist: Vec<i32>,
    queue: VecDeque<usize>,
    visited: Vec<usize>,
}

impl ClosenessState {
    fn new(node_count: usize) -> Self {
        Self {
            dist: vec![-1; node_count],
            queue: VecDeque::with_capacity(node_count),
            visited: Vec::with_capacity(node_count),
        }
    }

    fn harmonic_sum(&mut self, source: usize, adjacency: &[Vec<usize>]) -> f32 {
        self.reset();
        self.dist[source] = 0;
        self.visited.push(source);
        self.queue.push_back(source);

        let mut harmonic = 0.0;
        while let Some(node) = self.queue.pop_front() {
            let depth = self.dist[node];
            for &neighbor in &adjacency[node] {
                if self.dist[neighbor] < 0 {
                    let next_depth = depth + 1;
                    self.dist[neighbor] = next_depth;
                    self.visited.push(neighbor);
                    harmonic += 1.0 / next_depth as f32;
                    self.queue.push_back(neighbor);
                }
            }
        }
        harmonic
    }

    fn reset(&mut self) {
        for &node in &self.visited {
            self.dist[node] = -1;
        }
        self.visited.clear();
        self.queue.clear();
    }
}

struct PageRankState {
    rank: Vec<f32>,
    next_rank: Vec<f32>,
    teleport: f32,
    damping: f32,
}

impl PageRankState {
    fn new(node_count: usize, damping: f32) -> Self {
        Self {
            rank: vec![1.0 / node_count as f32; node_count],
            next_rank: vec![0.0; node_count],
            teleport: (1.0 - damping) / node_count as f32,
            damping,
        }
    }

    fn iterate(&mut self, in_edges: &[Vec<(usize, f32)>], dangling: &[usize]) -> f32 {
        let dangling_sum: f32 = dangling.iter().map(|&index| self.rank[index]).sum();
        let dangling_contrib = self.damping * dangling_sum / self.rank.len() as f32;

        for (node, incoming) in in_edges.iter().enumerate() {
            let link_sum: f32 = incoming
                .iter()
                .map(|&(source, weight)| self.rank[source] * weight)
                .sum();
            self.next_rank[node] = self.teleport + dangling_contrib + self.damping * link_sum;
        }

        let delta: f32 = self
            .rank
            .iter()
            .zip(&self.next_rank)
            .map(|(current, next)| (current - next).abs())
            .sum();
        self.rank.copy_from_slice(&self.next_rank);
        delta
    }
}
