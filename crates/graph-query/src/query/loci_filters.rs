use graph_core::{Locus, LocusId, LocusKindId};
use rustc_hash::FxHashSet;

use super::LociQuery;

impl<'w> LociQuery<'w> {
    /// Keep only loci of the given kind.
    pub fn of_kind(mut self, kind: LocusKindId) -> Self {
        self.loci.retain(|l| l.kind == kind);
        self
    }

    /// Keep only loci where `state[slot]` satisfies `pred`.
    ///
    /// Loci whose state vector is shorter than `slot + 1` are excluded.
    pub fn where_state(mut self, slot: usize, pred: impl Fn(f32) -> bool) -> Self {
        self.loci
            .retain(|l| l.state.as_slice().get(slot).is_some_and(|&v| pred(v)));
        self
    }

    /// Keep only loci that have string property `key` satisfying `pred`.
    pub fn where_str_property(mut self, key: &str, pred: impl Fn(&str) -> bool) -> Self {
        self.loci.retain(|l| {
            self.world
                .properties()
                .get(l.id)
                .and_then(|p| p.get_str(key))
                .is_some_and(&pred)
        });
        self
    }

    /// Keep only loci that have numeric property `key` satisfying `pred`.
    pub fn where_f64_property(mut self, key: &str, pred: impl Fn(f64) -> bool) -> Self {
        self.loci.retain(|l| {
            self.world
                .properties()
                .get(l.id)
                .and_then(|p| p.get_f64(key))
                .is_some_and(&pred)
        });
        self
    }

    /// Keep only loci matching a custom predicate.
    pub fn matching(mut self, pred: impl Fn(&Locus) -> bool) -> Self {
        self.loci.retain(|l| pred(l));
        self
    }

    /// Keep only loci with total degree ≥ `min`.
    pub fn min_degree(mut self, min: usize) -> Self {
        self.loci.retain(|l| self.world.degree(l.id) >= min);
        self
    }

    /// Keep only loci that are reachable from `start` within `depth`
    /// undirected hops (excluding `start` itself).
    pub fn reachable_from(self, start: LocusId, depth: usize) -> Self {
        let reachable: FxHashSet<LocusId> =
            crate::traversal::reachable_from(self.world, start, depth)
                .into_iter()
                .collect();
        Self {
            world: self.world,
            loci: self
                .loci
                .into_iter()
                .filter(|l| reachable.contains(&l.id))
                .collect(),
        }
    }

    /// Keep only loci reachable by following edges **forward** from `start`
    /// within `depth` directed hops.
    pub fn downstream_of(self, start: LocusId, depth: usize) -> Self {
        let reachable: FxHashSet<LocusId> =
            crate::traversal::downstream_of(self.world, start, depth)
                .into_iter()
                .collect();
        Self {
            world: self.world,
            loci: self
                .loci
                .into_iter()
                .filter(|l| reachable.contains(&l.id))
                .collect(),
        }
    }

    /// Keep only loci reachable by following edges **backward** from `start`
    /// within `depth` directed hops.
    pub fn upstream_of(self, start: LocusId, depth: usize) -> Self {
        let reachable: FxHashSet<LocusId> = crate::traversal::upstream_of(self.world, start, depth)
            .into_iter()
            .collect();
        Self {
            world: self.world,
            loci: self
                .loci
                .into_iter()
                .filter(|l| reachable.contains(&l.id))
                .collect(),
        }
    }

    /// Keep the top `n` loci by `state[slot]` in descending order.
    pub fn top_n_by_state(mut self, slot: usize, n: usize) -> Self {
        self = self.sort_by_state(slot);
        self.loci.truncate(n);
        self
    }

    /// Sort the current set by `state[slot]` descending (no truncation).
    pub fn sort_by_state(mut self, slot: usize) -> Self {
        self.loci.sort_unstable_by(|a, b| {
            let lhs = a
                .state
                .as_slice()
                .get(slot)
                .copied()
                .unwrap_or(f32::NEG_INFINITY);
            let rhs = b
                .state
                .as_slice()
                .get(slot)
                .copied()
                .unwrap_or(f32::NEG_INFINITY);
            rhs.total_cmp(&lhs)
        });
        self
    }

    /// Keep the top `n` loci by total degree (most-connected first).
    pub fn top_n_by_degree(mut self, n: usize) -> Self {
        self.loci
            .sort_unstable_by_key(|l| std::cmp::Reverse(self.world.degree(l.id)));
        self.loci.truncate(n);
        self
    }
}
