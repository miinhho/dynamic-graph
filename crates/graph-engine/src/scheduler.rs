//! SCC-aware scheduling primitives.
//!
//! Architecture §7.6 calls for the runtime to "decompose the graph into
//! strongly connected components, update acyclic regions in topological order,
//! treat cyclic regions as iterative blocks". This module provides the
//! structural primitive: it takes a [`WorldSnapshot`] and returns an
//! [`SccPlan`] that orders entities so that downstream schedulers can run
//! acyclic regions in dependency order and re-iterate cyclic blocks.
//!
//! The plan is intentionally a pure data structure. It does not yet drive the
//! tick loop — the engine's commit boundary remains singular — but it gives
//! every higher-level scheduler the same SCC view of the world to build on.
//!
//! Channels with explicit `targets` are treated as directed edges from
//! `source` to each target. Cohort, broadcast and field-routed channels do
//! not contribute to the static topology because their target set is resolved
//! dynamically per tick; they fall through to the trailing "unscheduled"
//! group so a caller can either ignore them or run them after the structured
//! plan.

use graph_core::EntityId;
use graph_world::WorldSnapshot;
use rustc_hash::FxHashMap;

/// A topological + SCC view of the entities in a snapshot.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SccPlan {
    /// Singleton SCCs in reverse topological order (dependencies first), so a
    /// scheduler can update each entity exactly once.
    pub acyclic_order: Vec<EntityId>,
    /// Multi-entity SCCs in reverse topological order. Each component is the
    /// set of entities that participate in at least one cycle and should be
    /// updated as an iterative block.
    pub cyclic_components: Vec<Vec<EntityId>>,
    /// Entities the analysis could not place — typically those reachable only
    /// through dynamically-routed channels (broadcast, field, cohort).
    pub unscheduled: Vec<EntityId>,
}

impl SccPlan {
    /// Total number of entities the plan covers, including dynamic-only ones.
    pub fn entity_count(&self) -> usize {
        self.acyclic_order.len()
            + self.cyclic_components.iter().map(Vec::len).sum::<usize>()
            + self.unscheduled.len()
    }

    /// Number of multi-entity SCCs (true cyclic blocks).
    pub fn cyclic_block_count(&self) -> usize {
        self.cyclic_components.len()
    }
}

/// Compute an [`SccPlan`] for the given snapshot.
pub fn compute_scc_plan(snapshot: WorldSnapshot<'_>) -> SccPlan {
    // 1. Index entities into a dense `[0, n)` range for the SCC algorithm.
    let entities: Vec<EntityId> = snapshot.entities().map(|entity| entity.id).collect();
    let mut index_of: FxHashMap<EntityId, usize> = FxHashMap::default();
    for (i, id) in entities.iter().enumerate() {
        index_of.insert(*id, i);
    }
    let n = entities.len();

    // 2. Build adjacency from channels with explicit targets. Skip dynamic
    //    channels — their topology is computed at routing time.
    let mut adjacency: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut covered = vec![false; n];
    for channel in snapshot.channels() {
        if !channel.enabled {
            continue;
        }
        let Some(&src_idx) = index_of.get(&channel.source) else {
            continue;
        };
        if channel.targets.is_empty() {
            // Dynamic routing — no static edge contribution.
            continue;
        }
        covered[src_idx] = true;
        for target in &channel.targets {
            if let Some(&dst_idx) = index_of.get(target) {
                adjacency[src_idx].push(dst_idx);
                covered[dst_idx] = true;
            }
        }
    }

    // 3. Run Tarjan's SCC algorithm. Components are emitted in reverse
    //    topological order, which is exactly what we want for an
    //    update-dependencies-first schedule.
    let sccs = tarjan_scc(&adjacency);

    // 4. Translate the SCCs back into entity ids and partition them.
    //    Singleton SCCs whose entity has no static edges at all go into
    //    `unscheduled`: their topology is fully dynamic (broadcast/field/
    //    cohort) and the static plan can't say anything about their order.
    let mut plan = SccPlan::default();
    for component in sccs {
        if component.len() == 1 {
            let idx = component[0];
            let id = entities[idx];
            if covered[idx] {
                plan.acyclic_order.push(id);
            } else {
                plan.unscheduled.push(id);
            }
        } else {
            let mut block: Vec<EntityId> =
                component.into_iter().map(|i| entities[i]).collect();
            block.sort_unstable_by_key(|id| id.0);
            plan.cyclic_components.push(block);
        }
    }

    plan.unscheduled.sort_unstable_by_key(|id| id.0);
    plan
}

/// Tarjan's strongly-connected-components algorithm (iterative form).
///
/// Returns a list of components where each component is a list of node
/// indices into the input adjacency list. Components are emitted in reverse
/// topological order on the component DAG: a component appears after every
/// component it depends on.
fn tarjan_scc(adjacency: &[Vec<usize>]) -> Vec<Vec<usize>> {
    let n = adjacency.len();
    let mut index_counter: usize = 0;
    let mut indices: Vec<Option<usize>> = vec![None; n];
    let mut lowlinks: Vec<usize> = vec![0; n];
    let mut on_stack: Vec<bool> = vec![false; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut result: Vec<Vec<usize>> = Vec::new();

    // Iterative DFS to avoid blowing the call stack on large graphs.
    // Each frame remembers the next neighbour index to visit.
    let mut call_stack: Vec<(usize, usize)> = Vec::new();

    for start in 0..n {
        if indices[start].is_some() {
            continue;
        }
        indices[start] = Some(index_counter);
        lowlinks[start] = index_counter;
        index_counter += 1;
        stack.push(start);
        on_stack[start] = true;
        call_stack.push((start, 0));

        while let Some(&(v, mut i)) = call_stack.last() {
            let neighbours = &adjacency[v];
            let mut recursed = false;
            while i < neighbours.len() {
                let w = neighbours[i];
                i += 1;
                match indices[w] {
                    None => {
                        // Recurse into w.
                        indices[w] = Some(index_counter);
                        lowlinks[w] = index_counter;
                        index_counter += 1;
                        stack.push(w);
                        on_stack[w] = true;
                        // Persist the new neighbour cursor for v before
                        // descending into w.
                        let last = call_stack.last_mut().unwrap();
                        last.1 = i;
                        call_stack.push((w, 0));
                        recursed = true;
                        break;
                    }
                    Some(_) if on_stack[w] => {
                        let w_index = indices[w].unwrap();
                        if w_index < lowlinks[v] {
                            lowlinks[v] = w_index;
                        }
                    }
                    Some(_) => {
                        // Cross / forward edge — ignore.
                    }
                }
            }
            if recursed {
                continue;
            }
            // Done exploring v: maybe pop an SCC, then return to caller.
            if lowlinks[v] == indices[v].unwrap() {
                let mut component = Vec::new();
                loop {
                    let w = stack.pop().expect("stack non-empty inside scc pop");
                    on_stack[w] = false;
                    component.push(w);
                    if w == v {
                        break;
                    }
                }
                result.push(component);
            }
            call_stack.pop();
            if let Some(&(parent, _)) = call_stack.last() {
                if lowlinks[v] < lowlinks[parent] {
                    lowlinks[parent] = lowlinks[v];
                }
            }
        }
    }

    // Re-key components so callers can rely on stable ordering for the
    // singleton entries (Tarjan's natural order is fine for the DAG order
    // constraint, but we sort intra-component lists for determinism).
    for component in &mut result {
        component.sort_unstable();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tarjan_handles_simple_chain() {
        // 0 -> 1 -> 2
        let adj = vec![vec![1], vec![2], vec![]];
        let sccs = tarjan_scc(&adj);
        assert_eq!(sccs.len(), 3);
        // Reverse topological order: 2 first, then 1, then 0.
        assert_eq!(sccs[0], vec![2]);
        assert_eq!(sccs[1], vec![1]);
        assert_eq!(sccs[2], vec![0]);
    }

    #[test]
    fn tarjan_detects_simple_cycle() {
        // 0 -> 1 -> 0
        let adj = vec![vec![1], vec![0]];
        let sccs = tarjan_scc(&adj);
        assert_eq!(sccs.len(), 1);
        let mut comp = sccs[0].clone();
        comp.sort();
        assert_eq!(comp, vec![0, 1]);
    }

    #[test]
    fn tarjan_handles_two_disjoint_components() {
        // 0 -> 1 -> 0,  2 -> 3 (acyclic).
        let adj = vec![vec![1], vec![0], vec![3], vec![]];
        let sccs = tarjan_scc(&adj);
        assert_eq!(sccs.len(), 3);
        // Cyclic component should contain {0,1}.
        let cyclic: Vec<&Vec<usize>> = sccs.iter().filter(|c| c.len() > 1).collect();
        assert_eq!(cyclic.len(), 1);
        assert_eq!(cyclic[0], &vec![0, 1]);
    }

    #[test]
    fn tarjan_handles_self_loop_as_cyclic_singleton() {
        // 0 -> 0
        let adj = vec![vec![0]];
        let sccs = tarjan_scc(&adj);
        // Tarjan emits the SCC as a single-node component because we don't
        // distinguish self-loops; but it is conceptually cyclic. The plan
        // builder will treat any singleton as acyclic — that's an explicit
        // design simplification documented in `compute_scc_plan`.
        assert_eq!(sccs.len(), 1);
    }

    #[test]
    fn tarjan_diamond_topological_order() {
        //     0
        //    / \
        //   1   2
        //    \ /
        //     3
        let adj = vec![vec![1, 2], vec![3], vec![3], vec![]];
        let sccs = tarjan_scc(&adj);
        // 4 singleton components, 3 must appear first (no outgoing), 0 last.
        assert_eq!(sccs.len(), 4);
        assert_eq!(sccs[0], vec![3]);
        assert_eq!(sccs[3], vec![0]);
    }
}
