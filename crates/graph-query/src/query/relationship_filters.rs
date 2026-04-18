use graph_core::{BatchId, Endpoints, InfluenceKindId, LocusId, Relationship};
use rustc_hash::FxHashSet;

use super::RelationshipsQuery;

impl<'w> RelationshipsQuery<'w> {
    /// Keep only relationships of the given influence kind.
    pub fn of_kind(mut self, kind: InfluenceKindId) -> Self {
        self.rels.retain(|r| r.kind == kind);
        self
    }

    /// Keep only directed relationships whose source is `locus`.
    pub fn from(mut self, locus: LocusId) -> Self {
        self.rels
            .retain(|r| matches!(r.endpoints, Endpoints::Directed { from, .. } if from == locus));
        self
    }

    /// Keep only directed relationships whose target is `locus`.
    pub fn to(mut self, locus: LocusId) -> Self {
        self.rels
            .retain(|r| matches!(r.endpoints, Endpoints::Directed { to, .. } if to == locus));
        self
    }

    /// Keep only relationships that involve `locus` at either endpoint.
    pub fn touching(mut self, locus: LocusId) -> Self {
        self.rels.retain(|r| r.endpoints.involves(locus));
        self
    }

    /// Keep only relationships connecting `a` and `b` (direction-agnostic).
    pub fn between(mut self, a: LocusId, b: LocusId) -> Self {
        self.rels
            .retain(|r| r.endpoints.involves(a) && r.endpoints.involves(b));
        self
    }

    /// Keep only relationships whose activity exceeds `threshold`.
    pub fn above_activity(mut self, threshold: f32) -> Self {
        self.rels.retain(|r| r.activity() > threshold);
        self
    }

    /// Keep only relationships whose combined strength exceeds `threshold`.
    pub fn above_strength(mut self, threshold: f32) -> Self {
        self.rels.retain(|r| r.strength() > threshold);
        self
    }

    /// Keep only relationships where `state[slot_idx]` satisfies `pred`.
    pub fn where_slot(mut self, slot_idx: usize, pred: impl Fn(f32) -> bool) -> Self {
        self.rels
            .retain(|r| r.state.as_slice().get(slot_idx).is_some_and(|&v| pred(v)));
        self
    }

    /// Keep only relationships matching a custom predicate.
    pub fn matching(mut self, pred: impl Fn(&Relationship) -> bool) -> Self {
        self.rels.retain(|r| pred(r));
        self
    }

    /// Keep only directed relationships whose source is any of `loci`.
    pub fn from_any(mut self, loci: &[LocusId]) -> Self {
        let loci: FxHashSet<LocusId> = loci.iter().copied().collect();
        self.rels.retain(
            |r| matches!(r.endpoints, Endpoints::Directed { from, .. } if loci.contains(&from)),
        );
        self
    }

    /// Keep only directed relationships whose target is any of `loci`.
    pub fn to_any(mut self, loci: &[LocusId]) -> Self {
        let loci: FxHashSet<LocusId> = loci.iter().copied().collect();
        self.rels.retain(
            |r| matches!(r.endpoints, Endpoints::Directed { to, .. } if loci.contains(&to)),
        );
        self
    }

    /// Keep only relationships that involve any of `loci` at either endpoint.
    pub fn touching_any(mut self, loci: &[LocusId]) -> Self {
        let loci: FxHashSet<LocusId> = loci.iter().copied().collect();
        self.rels
            .retain(|r| loci.iter().any(|&id| r.endpoints.involves(id)));
        self
    }

    /// Keep only relationships created within the inclusive batch range `[from, to]`.
    pub fn created_in(mut self, from: BatchId, to: BatchId) -> Self {
        self.rels
            .retain(|r| r.created_batch >= from && r.created_batch <= to);
        self
    }

    /// Keep only relationships whose age is at least `min_batches`.
    pub fn older_than(mut self, current_batch: BatchId, min_batches: u64) -> Self {
        self.rels
            .retain(|r| r.age_in_batches(current_batch) >= min_batches);
        self
    }

    /// Keep only relationships idle for at least `min_batches` batches.
    pub fn idle_for(mut self, current_batch: BatchId, min_batches: u64) -> Self {
        self.rels
            .retain(|r| current_batch.0.saturating_sub(r.last_decayed_batch) >= min_batches);
        self
    }

    /// Keep the top `n` relationships by strength.
    pub fn top_n_by_strength(mut self, n: usize) -> Self {
        self.rels
            .sort_unstable_by(|a, b| b.strength().total_cmp(&a.strength()));
        self.rels.truncate(n);
        self
    }

    /// Keep the top `n` relationships by activity.
    pub fn top_n_by_activity(mut self, n: usize) -> Self {
        self.rels
            .sort_unstable_by(|a, b| b.activity().total_cmp(&a.activity()));
        self.rels.truncate(n);
        self
    }

    /// Keep the top `n` relationships by `change_count`.
    pub fn top_n_by_change_count(mut self, n: usize) -> Self {
        self.rels
            .sort_unstable_by_key(|r| std::cmp::Reverse(r.lineage.change_count));
        self.rels.truncate(n);
        self
    }
}
