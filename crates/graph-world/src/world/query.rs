//! Read-only query conveniences on `World`.
//!
//! These methods delegate to the underlying stores and provide a
//! unified query surface for the engine and external callers.

mod entity_queries;
mod relationship_queries;
mod temporal;

use graph_core::{
    BatchId, Change, ChangeId, Entity, EntityId, EntityLayer, Locus, LocusId, Relationship,
    RelationshipId, RelationshipKindId, StateVector, TrimSummary,
};
use rustc_hash::FxHashSet;

use self::entity_queries::{
    entities_at_batch, entity_member_relationships, entity_members, entity_of,
};
use self::relationship_queries::{
    induced_subgraph, relationships_active_above, relationships_between_of_kind,
    relationships_for_locus_of_kind, relationships_from_of_kind, relationships_to_of_kind,
};
use self::temporal::{locus_state_at, relationship_state_at, relationships_at_batch};
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

    /// Trim summaries for `locus`, oldest first (E2).
    ///
    /// Non-empty after `trim_before_batch` has been called and the locus had
    /// changes in the trimmed range. Use `causal_coarse_trail` in `graph-query`
    /// to follow these summaries across the trim boundary.
    pub fn trim_summaries_for_locus(&self, locus: LocusId) -> &[TrimSummary] {
        self.log.trim_summaries_for_locus(locus)
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
        relationships_for_locus_of_kind(self, locus, kind)
    }

    /// Directed outgoing relationships of a specific kind from `locus`. O(k).
    pub fn relationships_from_of_kind(
        &self,
        locus: LocusId,
        kind: RelationshipKindId,
    ) -> impl Iterator<Item = &Relationship> {
        relationships_from_of_kind(self, locus, kind)
    }

    /// Directed incoming relationships of a specific kind to `locus`. O(k).
    pub fn relationships_to_of_kind(
        &self,
        locus: LocusId,
        kind: RelationshipKindId,
    ) -> impl Iterator<Item = &Relationship> {
        relationships_to_of_kind(self, locus, kind)
    }

    /// All relationships between `a` and `b` of a specific kind
    /// (regardless of direction). O(k_a).
    pub fn relationships_between_of_kind(
        &self,
        a: LocusId,
        b: LocusId,
        kind: RelationshipKindId,
    ) -> impl Iterator<Item = &Relationship> {
        relationships_between_of_kind(self, a, b, kind)
    }

    /// All relationships whose current activity score exceeds `threshold`.
    ///
    /// O(R) where R = total relationships.
    pub fn relationships_active_above(
        &self,
        threshold: f32,
    ) -> impl Iterator<Item = &Relationship> {
        relationships_active_above(self, threshold)
    }

    /// All relationships whose **both** endpoints are members of `loci`
    /// (the induced subgraph of `loci` in the relationship graph).
    ///
    /// Complexity: O(Σ k_i) where k_i is the degree of locus i.
    pub fn induced_subgraph<'a>(&'a self, loci: &[LocusId]) -> Vec<&'a Relationship> {
        induced_subgraph(self, loci)
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

    // ── Temporal state reconstruction ───────────────────────────────────

    /// The locus's state vector as it was at `batch`, reconstructed from
    /// the change log (newest-first scan, stops at the first change whose
    /// `batch` ≤ target).
    ///
    /// Returns `None` when:
    /// - The locus has no recorded changes at or before `batch`.
    /// - All relevant history was trimmed from the log.
    pub fn locus_state_at(&self, locus: LocusId, batch: BatchId) -> Option<&StateVector> {
        locus_state_at(self, locus, batch)
    }

    /// The relationship's state vector as it was at `batch`, reconstructed
    /// from the change log.
    ///
    /// Returns `None` when the relationship has no recorded changes at or
    /// before `batch` (including when history was trimmed).
    pub fn relationship_state_at(
        &self,
        rel: RelationshipId,
        batch: BatchId,
    ) -> Option<&StateVector> {
        relationship_state_at(self, rel, batch)
    }

    // ── Entity reverse lookup ────────────────────────────────────────────

    /// The active entity whose current member set contains `locus`, if any.
    ///
    /// Complexity: O(E × M_avg) where E is the number of active entities
    /// and M_avg is the average member count. No permanent reverse index is
    /// maintained because entity membership changes frequently; callers that
    /// need repeated lookups should build their own index from `entities()`.
    pub fn entity_of(&self, locus: LocusId) -> Option<&Entity> {
        entity_of(self, locus)
    }

    // ── Entity member queries ────────────────────────────────────────────

    /// Loci that are currently members of `entity` (per its top layer).
    pub fn entity_members(&self, id: EntityId) -> impl Iterator<Item = &Locus> {
        entity_members(self, id)
    }

    /// Relationships that are currently part of `entity` (per its top layer).
    pub fn entity_member_relationships(&self, id: EntityId) -> impl Iterator<Item = &Relationship> {
        entity_member_relationships(self, id)
    }

    /// Most recent entity layer at or before `batch`.
    pub fn entity_at_batch(&self, id: EntityId, batch: BatchId) -> Option<&EntityLayer> {
        self.entities.layer_at_batch(id, batch)
    }

    // ── Point-in-time queries ────────────────────────────────────────────

    /// Reconstruct the entity landscape at a past batch.
    pub fn entities_at_batch(&self, batch: BatchId) -> Vec<(EntityId, &EntityLayer)> {
        entities_at_batch(self, batch)
    }

    /// Count of active relationships at a past batch by scanning the
    /// change log for relationship-subject changes.
    pub fn relationships_at_batch(&self, batch: BatchId) -> FxHashSet<RelationshipId> {
        relationships_at_batch(self, batch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{
        BatchId, Change, ChangeId, ChangeSubject, Entity, EntitySnapshot, InfluenceKindId, Locus,
        LocusId, LocusKindId, RelationshipId, StateVector,
    };

    fn push_locus_change(
        world: &mut World,
        id: u64,
        locus: u64,
        after: f32,
        batch: u64,
    ) -> ChangeId {
        let cid = ChangeId(id);
        world.log_mut().append(Change {
            id: cid,
            subject: ChangeSubject::Locus(LocusId(locus)),
            kind: InfluenceKindId(1),
            predecessors: vec![],
            before: StateVector::zeros(1),
            after: StateVector::from_slice(&[after]),
            batch: BatchId(batch),
            wall_time: None,
            metadata: None,
        });
        cid
    }

    fn push_rel_change(
        world: &mut World,
        id: u64,
        rel: u64,
        activity: f32,
        batch: u64,
    ) -> ChangeId {
        let cid = ChangeId(id);
        world.log_mut().append(Change {
            id: cid,
            subject: ChangeSubject::Relationship(RelationshipId(rel)),
            kind: InfluenceKindId(1),
            predecessors: vec![],
            before: StateVector::zeros(2),
            after: StateVector::from_slice(&[activity, 0.5]),
            batch: BatchId(batch),
            wall_time: None,
            metadata: None,
        });
        cid
    }

    #[test]
    fn locus_state_at_returns_most_recent_before_target() {
        let mut w = World::new();
        push_locus_change(&mut w, 0, 0, 0.2, 1);
        push_locus_change(&mut w, 1, 0, 0.5, 3);
        push_locus_change(&mut w, 2, 0, 0.9, 5);

        // batch 4 → should see change at batch 3 (state = 0.5)
        let state = w.locus_state_at(LocusId(0), BatchId(4)).unwrap();
        assert!(
            (state.as_slice()[0] - 0.5).abs() < 1e-5,
            "expected 0.5, got {:?}",
            state
        );
    }

    #[test]
    fn locus_state_at_exact_batch_match() {
        let mut w = World::new();
        push_locus_change(&mut w, 0, 0, 0.7, 2);

        let state = w.locus_state_at(LocusId(0), BatchId(2)).unwrap();
        assert!((state.as_slice()[0] - 0.7).abs() < 1e-5);
    }

    #[test]
    fn locus_state_at_returns_none_before_first_change() {
        let mut w = World::new();
        push_locus_change(&mut w, 0, 0, 0.5, 5);

        assert!(w.locus_state_at(LocusId(0), BatchId(4)).is_none());
    }

    #[test]
    fn relationship_state_at_reconstructs_correctly() {
        let mut w = World::new();
        push_rel_change(&mut w, 0, 42, 0.3, 1);
        push_rel_change(&mut w, 1, 42, 0.6, 4);

        let state = w
            .relationship_state_at(RelationshipId(42), BatchId(3))
            .unwrap();
        assert!((state.as_slice()[0] - 0.3).abs() < 1e-5);
    }

    fn add_rel(world: &mut World, from: u64, to: u64, kind: u64) -> RelationshipId {
        use graph_core::{Endpoints, StateVector};
        world.add_relationship(
            Endpoints::directed(LocusId(from), LocusId(to)),
            InfluenceKindId(kind),
            StateVector::from_slice(&[1.0, 0.0]),
        )
    }

    #[test]
    fn relationships_from_of_kind_filters_correctly() {
        let mut w = World::new();
        let r1 = add_rel(&mut w, 1, 2, 10);
        let _r2 = add_rel(&mut w, 1, 3, 20); // different kind
        let _r3 = add_rel(&mut w, 4, 1, 10); // incoming, not outgoing

        let from_kind10: Vec<_> = w
            .relationships_from_of_kind(LocusId(1), InfluenceKindId(10))
            .collect();
        assert_eq!(from_kind10.len(), 1);
        assert_eq!(from_kind10[0].id, r1);
    }

    #[test]
    fn relationships_to_of_kind_filters_correctly() {
        let mut w = World::new();
        let _r1 = add_rel(&mut w, 1, 2, 10);
        let r2 = add_rel(&mut w, 3, 2, 10);
        let _r3 = add_rel(&mut w, 3, 2, 20); // different kind

        let to_kind10: Vec<_> = w
            .relationships_to_of_kind(LocusId(2), InfluenceKindId(10))
            .collect();
        // r1 and r2 both arrive at L2 with kind 10
        assert_eq!(to_kind10.len(), 2);
        assert!(to_kind10.iter().any(|r| r.id == r2));
    }

    #[test]
    fn relationships_between_of_kind_filters_correctly() {
        let mut w = World::new();
        let r1 = add_rel(&mut w, 1, 2, 10);
        let _r2 = add_rel(&mut w, 1, 2, 20); // same pair, different kind
        let _r3 = add_rel(&mut w, 1, 3, 10); // different pair

        let between_kind10: Vec<_> = w
            .relationships_between_of_kind(LocusId(1), LocusId(2), InfluenceKindId(10))
            .collect();
        assert_eq!(between_kind10.len(), 1);
        assert_eq!(between_kind10[0].id, r1);
    }

    #[test]
    fn entity_of_finds_containing_entity() {
        let mut w = World::new();
        w.insert_locus(Locus::new(
            LocusId(0),
            LocusKindId(1),
            StateVector::zeros(1),
        ));
        w.insert_locus(Locus::new(
            LocusId(1),
            LocusKindId(1),
            StateVector::zeros(1),
        ));

        let eid = w.entities_mut().mint_id();
        w.entities_mut().insert(Entity::born(
            eid,
            BatchId(1),
            EntitySnapshot {
                members: vec![LocusId(0)],
                member_relationships: vec![],
                coherence: 0.9,
            },
        ));

        let found = w.entity_of(LocusId(0)).unwrap();
        assert_eq!(found.id, eid);
        assert!(w.entity_of(LocusId(1)).is_none());
    }
}
