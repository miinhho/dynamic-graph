use std::collections::VecDeque;

use graph_core::LocusId;
use graph_world::World;

use super::indexed::{IndexedLoci, index_loci, unweighted_undirected_adj};

pub(super) fn all_betweenness_inner(world: &World) -> Vec<(LocusId, f32)> {
    let IndexedLoci { loci, idx } = index_loci(world);
    let node_count = loci.len();
    if node_count < 3 {
        return loci.into_iter().map(|id| (id, 0.0)).collect();
    }

    let adjacency = unweighted_undirected_adj(world, &idx, node_count);
    let norm = (node_count - 1) as f32 * (node_count - 2) as f32;
    let mut state = BrandesState::new(node_count);

    for source in 0..node_count {
        state.run_source(source, &adjacency);
    }

    loci.into_iter()
        .enumerate()
        .map(|(index, id)| (id, state.centrality[index] / norm))
        .collect()
}

struct BrandesState {
    centrality: Vec<f32>,
    stack: Vec<usize>,
    queue: VecDeque<usize>,
    visited: Vec<usize>,
    sigma: Vec<f32>,
    dist: Vec<i32>,
    delta: Vec<f32>,
    pred: Vec<Vec<usize>>,
}

impl BrandesState {
    fn new(node_count: usize) -> Self {
        Self {
            centrality: vec![0.0; node_count],
            stack: Vec::with_capacity(node_count),
            queue: VecDeque::with_capacity(node_count),
            visited: Vec::with_capacity(node_count),
            sigma: vec![0.0; node_count],
            dist: vec![-1; node_count],
            delta: vec![0.0; node_count],
            pred: vec![Vec::new(); node_count],
        }
    }

    fn run_source(&mut self, source: usize, adjacency: &[Vec<usize>]) {
        self.reset();
        self.initialize_source(source);
        self.forward_bfs(source, adjacency);
        self.accumulate_dependencies(source);
    }

    fn reset(&mut self) {
        for &node in &self.visited {
            self.sigma[node] = 0.0;
            self.dist[node] = -1;
            self.delta[node] = 0.0;
            self.pred[node].clear();
        }
        self.visited.clear();
        self.stack.clear();
        self.queue.clear();
    }

    fn initialize_source(&mut self, source: usize) {
        self.sigma[source] = 1.0;
        self.dist[source] = 0;
        self.visited.push(source);
        self.queue.push_back(source);
    }

    fn forward_bfs(&mut self, source: usize, adjacency: &[Vec<usize>]) {
        let _ = source;
        while let Some(node) = self.queue.pop_front() {
            self.stack.push(node);
            let node_dist = self.dist[node];
            for &neighbor in &adjacency[node] {
                self.discover_neighbor(node, neighbor, node_dist);
            }
        }
    }

    fn discover_neighbor(&mut self, node: usize, neighbor: usize, node_dist: i32) {
        if self.dist[neighbor] < 0 {
            self.dist[neighbor] = node_dist + 1;
            self.visited.push(neighbor);
            self.queue.push_back(neighbor);
        }
        if self.dist[neighbor] == node_dist + 1 {
            self.sigma[neighbor] += self.sigma[node];
            self.pred[neighbor].push(node);
        }
    }

    fn accumulate_dependencies(&mut self, source: usize) {
        while let Some(node) = self.stack.pop() {
            for &predecessor in &self.pred[node] {
                self.delta[predecessor] +=
                    (self.sigma[predecessor] / self.sigma[node]) * (1.0 + self.delta[node]);
            }
            if node != source {
                self.centrality[node] += self.delta[node];
            }
        }
    }
}
