use graph_core::{InfluenceKindId, LocusId, RelationshipId};
use rustc_hash::FxHashSet;

use super::SubscriptionStore;

impl SubscriptionStore {
    pub fn subscribers(&self, rel_id: RelationshipId) -> impl Iterator<Item = LocusId> + '_ {
        self.by_relationship
            .get(&rel_id)
            .into_iter()
            .flat_map(|set| set.iter().copied())
    }

    pub fn has_subscribers(&self, rel_id: RelationshipId) -> bool {
        self.by_relationship
            .get(&rel_id)
            .map(|s| !s.is_empty())
            .unwrap_or(false)
    }

    pub fn has_kind_subscribers(&self, kind: InfluenceKindId) -> bool {
        self.by_kind
            .get(&kind)
            .map(|s| !s.is_empty())
            .unwrap_or(false)
    }

    pub fn kind_subscribers(&self, kind: InfluenceKindId) -> impl Iterator<Item = LocusId> + '_ {
        self.by_kind
            .get(&kind)
            .into_iter()
            .flat_map(|s| s.iter().copied())
    }

    pub fn has_anchor_kind_subscribers(&self, anchor: LocusId, kind: InfluenceKindId) -> bool {
        self.by_anchor_kind
            .get(&(anchor, kind))
            .map(|s| !s.is_empty())
            .unwrap_or(false)
    }

    pub fn anchor_kind_subscribers(
        &self,
        anchor: LocusId,
        kind: InfluenceKindId,
    ) -> impl Iterator<Item = LocusId> + '_ {
        self.by_anchor_kind
            .get(&(anchor, kind))
            .into_iter()
            .flat_map(|s| s.iter().copied())
    }

    pub fn has_any_subscribers(
        &self,
        rel_id: RelationshipId,
        kind: InfluenceKindId,
        from: LocusId,
        to: LocusId,
    ) -> bool {
        self.has_subscribers(rel_id)
            || self.has_kind_subscribers(kind)
            || self.has_anchor_kind_subscribers(from, kind)
            || (from != to && self.has_anchor_kind_subscribers(to, kind))
    }

    pub fn collect_subscribers(
        &self,
        rel_id: RelationshipId,
        kind: InfluenceKindId,
        from: LocusId,
        to: LocusId,
    ) -> Vec<LocusId> {
        let mut seen: FxHashSet<LocusId> = FxHashSet::default();
        let mut out = Vec::new();
        for locus in self
            .subscribers(rel_id)
            .chain(self.kind_subscribers(kind))
            .chain(self.anchor_kind_subscribers(from, kind))
            .chain(self.anchor_kind_subscribers(to, kind))
        {
            if seen.insert(locus) {
                out.push(locus);
            }
        }
        out
    }
}
