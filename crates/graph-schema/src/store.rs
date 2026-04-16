//! [`DeclarationStore`]: append-only store for time-versioned facts.

use rustc_hash::FxHashMap;

use crate::fact::{DeclaredFact, DeclaredFactId, DeclaredRelKind};
use graph_core::LocusId;

/// Append-only store for [`DeclaredFact`]s with a monotone version counter.
///
/// ## Versioning model
///
/// Every mutation (`assert_fact`, `retract_fact`) increments an internal
/// **version counter**. `DeclaredFact::asserted_at` / `retracted_at` record
/// the version at which each fact was written or withdrawn. This makes all
/// point-in-time queries reproducible without tying them to wall-clock time
/// or the dynamic world's `BatchId`.
///
/// ## Indices
///
/// The store maintains two secondary indices for O(k) lookup:
/// - `by_subject`: `LocusId → [FactId]`
/// - `by_object`: `LocusId → [FactId]`
///
/// Facts are never physically deleted — retraction sets `retracted_at` only.
#[derive(Debug, Default, Clone)]
pub struct DeclarationStore {
    facts: Vec<DeclaredFact>,
    /// Subject locus → fact IDs (all versions, including retracted).
    by_subject: FxHashMap<LocusId, Vec<DeclaredFactId>>,
    /// Object locus → fact IDs (all versions, including retracted).
    by_object: FxHashMap<LocusId, Vec<DeclaredFactId>>,
    next_id: u64,
    /// Monotone counter incremented on every mutation.
    version: u64,
}

impl DeclarationStore {
    /// Current version of the store.
    #[inline]
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Total number of facts (including retracted).
    #[inline]
    pub fn len(&self) -> usize {
        self.facts.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.facts.is_empty()
    }

    // ── Mutations ────────────────────────────────────────────────────────────

    /// Assert a new fact. Returns the assigned [`DeclaredFactId`].
    ///
    /// Increments the store version. If an identical active fact (same subject,
    /// predicate, object) already exists it is returned without creating a
    /// duplicate.
    pub fn assert_fact(
        &mut self,
        subject: LocusId,
        predicate: DeclaredRelKind,
        object: LocusId,
    ) -> DeclaredFactId {
        // Idempotence: don't duplicate an active fact.
        if let Some(&id) = self.by_subject.get(&subject).and_then(|ids| {
            ids.iter().find(|&&fid| {
                let f = self.fact(fid).unwrap();
                f.is_active() && f.predicate == predicate && f.object == object
            })
        }) {
            return id;
        }

        self.version += 1;
        let id = DeclaredFactId(self.next_id);
        self.next_id += 1;

        let fact = DeclaredFact {
            id,
            subject,
            predicate,
            object,
            asserted_at: self.version,
            retracted_at: None,
        };

        self.by_subject.entry(subject).or_default().push(id);
        self.by_object.entry(object).or_default().push(id);
        self.facts.push(fact);
        id
    }

    /// Retract a fact by ID. No-op if the fact is already retracted or unknown.
    ///
    /// Increments the store version only when an active fact is retracted.
    pub fn retract_fact(&mut self, id: DeclaredFactId) {
        if let Some(f) = self.facts.iter_mut().find(|f| f.id == id && f.is_active()) {
            self.version += 1;
            f.retracted_at = Some(self.version);
        }
    }

    /// Retract all active facts between `subject` and `object` of a given
    /// predicate. Returns the number of facts retracted.
    pub fn retract_between(
        &mut self,
        subject: LocusId,
        predicate: &DeclaredRelKind,
        object: LocusId,
    ) -> usize {
        let ids: Vec<DeclaredFactId> = self
            .by_subject
            .get(&subject)
            .into_iter()
            .flat_map(|v| v.iter().copied())
            .filter(|&fid| {
                self.fact(fid).map_or(false, |f| {
                    f.is_active() && &f.predicate == predicate && f.object == object
                })
            })
            .collect();

        let count = ids.len();
        for id in ids {
            self.retract_fact(id);
        }
        count
    }

    // ── Queries ──────────────────────────────────────────────────────────────

    /// Retrieve a fact by ID, or `None` if not found.
    #[inline]
    pub fn fact(&self, id: DeclaredFactId) -> Option<&DeclaredFact> {
        self.facts.iter().find(|f| f.id == id)
    }

    /// All currently active facts (not retracted).
    pub fn active_facts(&self) -> impl Iterator<Item = &DeclaredFact> {
        self.facts.iter().filter(|f| f.is_active())
    }

    /// All facts active at a specific store version (including later-retracted).
    pub fn facts_at(&self, version: u64) -> impl Iterator<Item = &DeclaredFact> {
        self.facts.iter().filter(move |f| f.active_at(version))
    }

    /// Active facts originating from `subject`.
    pub fn facts_from(&self, subject: LocusId) -> impl Iterator<Item = &DeclaredFact> {
        self.by_subject
            .get(&subject)
            .into_iter()
            .flat_map(|ids| ids.iter())
            .filter_map(|&id| self.fact(id))
            .filter(|f| f.is_active())
    }

    /// Active facts pointing to `object`.
    pub fn facts_to(&self, object: LocusId) -> impl Iterator<Item = &DeclaredFact> {
        self.by_object
            .get(&object)
            .into_iter()
            .flat_map(|ids| ids.iter())
            .filter_map(|&id| self.fact(id))
            .filter(|f| f.is_active())
    }

    /// Active facts of a given predicate between `subject` and `object`.
    pub fn facts_between(
        &self,
        subject: LocusId,
        predicate: &DeclaredRelKind,
        object: LocusId,
    ) -> impl Iterator<Item = &DeclaredFact> {
        self.by_subject
            .get(&subject)
            .into_iter()
            .flat_map(|ids| ids.iter())
            .filter_map(|&id| self.fact(id))
            .filter(move |f| f.is_active() && &f.predicate == predicate && f.object == object)
    }

    /// All active predicates asserted between `subject` and `object`
    /// (bidirectional — also checks `object` as subject).
    pub fn predicates_between(
        &self,
        a: LocusId,
        b: LocusId,
    ) -> impl Iterator<Item = &DeclaredRelKind> {
        let ab = self
            .by_subject
            .get(&a)
            .into_iter()
            .flat_map(|ids| ids.iter())
            .filter_map(|&id| self.fact(id))
            .filter(move |f| f.is_active() && f.object == b)
            .map(|f| &f.predicate);
        let ba = self
            .by_subject
            .get(&b)
            .into_iter()
            .flat_map(|ids| ids.iter())
            .filter_map(|&id| self.fact(id))
            .filter(move |f| f.is_active() && f.object == a)
            .map(|f| &f.predicate);
        ab.chain(ba)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::LocusId;

    fn kind(s: &str) -> DeclaredRelKind {
        DeclaredRelKind::new(s)
    }

    #[test]
    fn assert_and_query_active_fact() {
        let mut store = DeclarationStore::default();
        let id = store.assert_fact(LocusId(1), kind("reports_to"), LocusId(2));
        let fact = store.fact(id).unwrap();
        assert_eq!(fact.subject, LocusId(1));
        assert_eq!(fact.object, LocusId(2));
        assert!(fact.is_active());
        assert_eq!(store.active_facts().count(), 1);
    }

    #[test]
    fn assert_idempotent_for_active_duplicate() {
        let mut store = DeclarationStore::default();
        let id1 = store.assert_fact(LocusId(1), kind("knows"), LocusId(2));
        let id2 = store.assert_fact(LocusId(1), kind("knows"), LocusId(2));
        assert_eq!(id1, id2);
        assert_eq!(store.active_facts().count(), 1);
    }

    #[test]
    fn retract_deactivates_fact() {
        let mut store = DeclarationStore::default();
        let id = store.assert_fact(LocusId(1), kind("manages"), LocusId(3));
        let v_before = store.version();
        store.retract_fact(id);
        assert!(!store.fact(id).unwrap().is_active());
        // Version advanced on retraction.
        assert!(store.version() > v_before);
    }

    #[test]
    fn point_in_time_query_via_facts_at() {
        let mut store = DeclarationStore::default();
        let id = store.assert_fact(LocusId(1), kind("leads"), LocusId(4));
        let v_assert = store.fact(id).unwrap().asserted_at;
        store.retract_fact(id);
        let v_retract = store.fact(id).unwrap().retracted_at.unwrap();

        assert_eq!(store.facts_at(v_assert).count(), 1);
        assert_eq!(store.facts_at(v_retract).count(), 0);
        assert_eq!(store.facts_at(v_assert - 1).count(), 0);
    }

    #[test]
    fn facts_from_filters_by_subject() {
        let mut store = DeclarationStore::default();
        store.assert_fact(LocusId(1), kind("knows"), LocusId(2));
        store.assert_fact(LocusId(1), kind("knows"), LocusId(3));
        store.assert_fact(LocusId(2), kind("knows"), LocusId(3));
        assert_eq!(store.facts_from(LocusId(1)).count(), 2);
        assert_eq!(store.facts_from(LocusId(2)).count(), 1);
    }

    #[test]
    fn retract_between_removes_matching_facts() {
        let mut store = DeclarationStore::default();
        store.assert_fact(LocusId(1), kind("likes"), LocusId(2));
        store.assert_fact(LocusId(1), kind("trusts"), LocusId(2));
        let removed = store.retract_between(LocusId(1), &kind("likes"), LocusId(2));
        assert_eq!(removed, 1);
        assert_eq!(store.active_facts().count(), 1);
        assert_eq!(store.active_facts().next().unwrap().predicate, kind("trusts"));
    }

    #[test]
    fn version_increments_on_each_mutation() {
        let mut store = DeclarationStore::default();
        assert_eq!(store.version(), 0);
        store.assert_fact(LocusId(1), kind("x"), LocusId(2));
        assert_eq!(store.version(), 1);
        let id = store.assert_fact(LocusId(3), kind("y"), LocusId(4));
        assert_eq!(store.version(), 2);
        store.retract_fact(id);
        assert_eq!(store.version(), 3);
    }

    #[test]
    fn idempotent_assert_does_not_increment_version() {
        let mut store = DeclarationStore::default();
        store.assert_fact(LocusId(1), kind("x"), LocusId(2));
        let v = store.version();
        store.assert_fact(LocusId(1), kind("x"), LocusId(2));
        assert_eq!(store.version(), v);
    }
}
