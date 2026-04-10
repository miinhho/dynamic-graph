//! Read-only query conveniences on `World`.
//!
//! These methods delegate to the underlying stores and provide a
//! unified query surface for the engine and external callers.

use graph_core::{
    BatchId, Change, ChangeId, EntityId, EntityLayer, Locus, LocusId,
    Relationship, RelationshipId, RelationshipKindId,
};
use rustc_hash::FxHashSet;

use super::World;

impl World {
    // ── Change log queries ───────────────────────────────────────────────

    /// Iterate changes to a locus, newest first. Delegates to
    /// `ChangeLog::changes_to_locus`; O(k) where k is the number of
    /// changes targeting this locus.
    pub fn changes_to_locus(&self, id: LocusId) -> impl Iterator<Item = &Change> {
        self.log.changes_to_locus(id)
    }

    /// Iterate changes to a relationship, newest first.
    pub fn changes_to_relationship(&self, id: RelationshipId) -> impl Iterator<Item = &Change> {
        self.log.changes_to_relationship(id)
    }

    /// Direct predecessor changes of `change_id`. Delegates to
    /// `ChangeLog::predecessors`.
    pub fn predecessors(&self, change_id: ChangeId) -> impl Iterator<Item = &Change> {
        self.log.predecessors(change_id)
    }

    /// All causal ancestors of `change_id` in BFS order. Delegates to
    /// `ChangeLog::causal_ancestors`.
    pub fn causal_ancestors(&self, change_id: ChangeId) -> Vec<&Change> {
        self.log.causal_ancestors(change_id)
    }

    /// Returns `true` if `ancestor` is a causal ancestor of `descendant`.
    /// Delegates to `ChangeLog::is_ancestor_of`.
    pub fn is_ancestor_of(&self, ancestor: ChangeId, descendant: ChangeId) -> bool {
        self.log.is_ancestor_of(ancestor, descendant)
    }

    // ── Relationship queries ─────────────────────────────────────────────

    /// All relationships that involve `locus` in any endpoint position.
    /// O(k) where k is the number of relationships at that locus.
    pub fn relationships_for_locus(&self, locus: LocusId) -> impl Iterator<Item = &Relationship> {
        self.relationships.relationships_for_locus(locus)
    }

    /// Directed relationships where `from == locus`. O(k).
    pub fn relationships_from(&self, locus: LocusId) -> impl Iterator<Item = &Relationship> {
        self.relationships.relationships_from(locus)
    }

    /// Directed relationships where `to == locus`. O(k).
    pub fn relationships_to(&self, locus: LocusId) -> impl Iterator<Item = &Relationship> {
        self.relationships.relationships_to(locus)
    }

    /// Relationships whose endpoints include both `a` and `b`
    /// (regardless of direction or kind). O(k_a).
    pub fn relationships_between(
        &self,
        a: LocusId,
        b: LocusId,
    ) -> impl Iterator<Item = &Relationship> {
        self.relationships.relationships_between(a, b)
    }

    /// All relationships of a specific kind involving `locus`. O(k).
    pub fn relationships_for_locus_of_kind(
        &self,
        locus: LocusId,
        kind: RelationshipKindId,
    ) -> impl Iterator<Item = &Relationship> {
        self.relationships
            .relationships_for_locus(locus)
            .filter(move |r| r.kind == kind)
    }

    /// All relationships whose current activity score exceeds `threshold`.
    ///
    /// O(R) where R = total relationships.
    pub fn relationships_active_above(&self, threshold: f32) -> impl Iterator<Item = &Relationship> {
        self.relationships.iter().filter(move |r| r.activity() > threshold)
    }

    /// All relationships whose **both** endpoints are members of `loci`
    /// (the induced subgraph of `loci` in the relationship graph).
    ///
    /// Complexity: O(Σ k_i) where k_i is the degree of locus i.
    pub fn induced_subgraph<'a>(&'a self, loci: &[LocusId]) -> Vec<&'a Relationship> {
        let loci_set: FxHashSet<LocusId> = loci.iter().copied().collect();
        let mut seen: FxHashSet<RelationshipId> = FxHashSet::default();
        let mut result = Vec::new();
        for &locus in loci {
            for rel in self.relationships.relationships_for_locus(locus) {
                if seen.insert(rel.id) && rel.endpoints.all_endpoints_in(&loci_set) {
                    result.push(rel);
                }
            }
        }
        result
    }

    // ── Batch-range diff ─────────────────────────────────────────────────

    /// Summarise what changed between `from_batch` (inclusive) and
    /// `self.current_batch()` (exclusive).
    pub fn diff_since(&self, from_batch: BatchId) -> crate::diff::WorldDiff {
        crate::diff::WorldDiff::compute(self, from_batch, self.current_batch)
    }

    /// Summarise what changed in the batch range `[from, to)`.
    pub fn diff_between(&self, from: BatchId, to: BatchId) -> crate::diff::WorldDiff {
        crate::diff::WorldDiff::compute(self, from, to)
    }

    // ── Degree centrality ────────────────────────────────────────────────

    /// Total number of relationships involving `locus`.
    /// O(1) via the `by_locus` reverse index.
    pub fn degree(&self, locus: LocusId) -> usize {
        self.relationships.degree(locus)
    }

    /// Directed in-degree of `locus`. O(k).
    pub fn in_degree(&self, locus: LocusId) -> usize {
        self.relationships.in_degree(locus)
    }

    /// Directed out-degree of `locus`. O(k).
    pub fn out_degree(&self, locus: LocusId) -> usize {
        self.relationships.out_degree(locus)
    }

    /// Iterator over `(LocusId, degree)` for every locus with at least
    /// one relationship.
    pub fn degree_iter(&self) -> impl Iterator<Item = (LocusId, usize)> + '_ {
        self.relationships.degree_iter()
    }

    // ── Aggregate snapshot ───────────────────────────────────────────────

    /// Compute a `WorldMetrics` snapshot.
    pub fn metrics(&self) -> crate::metrics::WorldMetrics {
        crate::metrics::WorldMetrics::compute(self)
    }

    // ── Entity member queries ────────────────────────────────────────────

    /// Loci that are currently members of `entity` (per its top layer).
    pub fn entity_members(&self, id: EntityId) -> impl Iterator<Item = &Locus> {
        self.entities
            .get(id)
            .map(|e| e.current.members.as_slice())
            .unwrap_or(&[])
            .iter()
            .filter_map(|&lid| self.loci.get(lid))
    }

    /// Relationships that are currently part of `entity` (per its top layer).
    pub fn entity_member_relationships(&self, id: EntityId) -> impl Iterator<Item = &Relationship> {
        self.entities
            .get(id)
            .map(|e| e.current.member_relationships.as_slice())
            .unwrap_or(&[])
            .iter()
            .filter_map(|&rid| self.relationships.get(rid))
    }

    /// Most recent entity layer at or before `batch`.
    pub fn entity_at_batch(&self, id: EntityId, batch: BatchId) -> Option<&EntityLayer> {
        self.entities.layer_at_batch(id, batch)
    }

    // ── Point-in-time queries ────────────────────────────────────────────

    /// Reconstruct the entity landscape at a past batch.
    pub fn entities_at_batch(&self, batch: BatchId) -> Vec<(EntityId, &EntityLayer)> {
        self.entities
            .iter()
            .filter_map(|e| {
                self.entities
                    .layer_at_batch(e.id, batch)
                    .map(|layer| (e.id, layer))
            })
            .collect()
    }

    /// Count of active relationships at a past batch by scanning the
    /// change log for relationship-subject changes.
    pub fn relationships_at_batch(&self, batch: BatchId) -> FxHashSet<RelationshipId> {
        let mut seen = FxHashSet::default();
        for change in self.log.iter() {
            if change.batch.0 > batch.0 {
                continue;
            }
            if let graph_core::ChangeSubject::Relationship(rid) = change.subject {
                seen.insert(rid);
            }
        }
        for rel in self.relationships.iter() {
            if rel.lineage.created_by.is_some() {
                seen.insert(rel.id);
            }
        }
        seen
    }
}
