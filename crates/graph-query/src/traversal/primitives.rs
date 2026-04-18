use std::collections::{BinaryHeap, VecDeque};

use graph_core::LocusId;
use rustc_hash::{FxHashMap, FxHashSet};

pub(super) fn reconstruct_path(
    from: LocusId,
    to: LocusId,
    prev: &FxHashMap<LocusId, LocusId>,
) -> Vec<LocusId> {
    let mut path = vec![to];
    let mut node = to;
    while node != from {
        debug_assert!(
            prev.contains_key(&node),
            "reconstruct_path: node {node:?} not in prev map — `to` must be reachable from `from`"
        );
        node = prev[&node];
        path.push(node);
    }
    path.reverse();
    path
}

pub(super) fn dijkstra_path(
    from: LocusId,
    to: LocusId,
    mut for_neighbors: impl FnMut(LocusId, &mut Vec<(LocusId, f32)>),
) -> Option<Vec<LocusId>> {
    #[derive(PartialEq)]
    struct Entry(f32, LocusId);
    impl Eq for Entry {}
    impl PartialOrd for Entry {
        fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
            Some(self.cmp(other))
        }
    }
    impl Ord for Entry {
        fn cmp(&self, other: &Self) -> std::cmp::Ordering {
            other.0.total_cmp(&self.0)
        }
    }

    let mut dist: FxHashMap<LocusId, f32> = FxHashMap::default();
    let mut prev: FxHashMap<LocusId, LocusId> = FxHashMap::default();
    let mut heap: BinaryHeap<Entry> = BinaryHeap::new();
    let mut buf: Vec<(LocusId, f32)> = Vec::new();

    dist.insert(from, 0.0);
    heap.push(Entry(0.0, from));

    while let Some(Entry(cost, current)) = heap.pop() {
        if current == to {
            return Some(reconstruct_path(from, to, &prev));
        }
        if dist.get(&current).is_some_and(|&d| cost > d) {
            continue;
        }
        buf.clear();
        for_neighbors(current, &mut buf);
        for &(neighbor, edge_cost) in &buf {
            let new_cost = cost + edge_cost;
            if dist.get(&neighbor).is_none_or(|&d| new_cost < d) {
                dist.insert(neighbor, new_cost);
                prev.insert(neighbor, current);
                heap.push(Entry(new_cost, neighbor));
            }
        }
    }
    None
}

pub(super) fn bfs_path(
    from: LocusId,
    to: LocusId,
    mut for_neighbors: impl FnMut(LocusId, &mut Vec<LocusId>),
) -> Option<Vec<LocusId>> {
    if from == to {
        return Some(vec![from]);
    }
    let mut prev: FxHashMap<LocusId, LocusId> = FxHashMap::default();
    let mut queue: VecDeque<LocusId> = VecDeque::new();
    let mut buf: Vec<LocusId> = Vec::new();
    prev.insert(from, from);
    queue.push_back(from);

    while let Some(current) = queue.pop_front() {
        buf.clear();
        for_neighbors(current, &mut buf);
        for &neighbor in &buf {
            if prev.contains_key(&neighbor) {
                continue;
            }
            prev.insert(neighbor, current);
            if neighbor == to {
                return Some(reconstruct_path(from, to, &prev));
            }
            queue.push_back(neighbor);
        }
    }
    None
}

pub(super) fn bfs_reachable(
    start: LocusId,
    depth: usize,
    mut for_neighbors: impl FnMut(LocusId, &mut Vec<LocusId>),
) -> Vec<LocusId> {
    if depth == 0 {
        return Vec::new();
    }
    let mut dist: FxHashMap<LocusId, usize> = FxHashMap::default();
    let mut queue: VecDeque<LocusId> = VecDeque::new();
    let mut buf: Vec<LocusId> = Vec::new();
    dist.insert(start, 0);
    queue.push_back(start);

    let mut result = Vec::new();
    while let Some(current) = queue.pop_front() {
        let d = dist[&current];
        if d >= depth {
            continue;
        }
        buf.clear();
        for_neighbors(current, &mut buf);
        for &neighbor in &buf {
            if dist.contains_key(&neighbor) {
                continue;
            }
            dist.insert(neighbor, d + 1);
            result.push(neighbor);
            queue.push_back(neighbor);
        }
    }
    result
}

pub(super) fn bfs_components(
    all_loci: &[LocusId],
    mut for_neighbors: impl FnMut(LocusId, &mut Vec<LocusId>),
) -> Vec<Vec<LocusId>> {
    let mut visited: FxHashSet<LocusId> = FxHashSet::default();
    let mut components: Vec<Vec<LocusId>> = Vec::new();
    let mut buf: Vec<LocusId> = Vec::new();

    for &seed in all_loci {
        if visited.contains(&seed) {
            continue;
        }
        let mut component = Vec::new();
        let mut queue: VecDeque<LocusId> = VecDeque::new();
        visited.insert(seed);
        queue.push_back(seed);
        while let Some(current) = queue.pop_front() {
            component.push(current);
            buf.clear();
            for_neighbors(current, &mut buf);
            for &neighbor in &buf {
                if visited.insert(neighbor) {
                    queue.push_back(neighbor);
                }
            }
        }
        components.push(component);
    }
    components
}
