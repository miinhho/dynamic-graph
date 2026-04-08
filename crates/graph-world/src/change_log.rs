//! Append-only log of committed changes.
//!
//! The change log is the substrate's only history. Higher layers
//! (relationships, entities) derive their state from it. The log is the
//! "raw change log" memory layer in `docs/redesign.md` §3.5 — fast
//! weathering will eventually trim its tail, but for now it grows
//! unbounded.
//!
//! Changes are pushed in the order they commit, which the engine
//! guarantees is consistent with their causal partial order: if A is in
//! B's predecessor set, A is recorded earlier.

use graph_core::{BatchId, Change, ChangeId, ChangeSubject, LocusId};

#[derive(Debug, Default, Clone)]
pub struct ChangeLog {
    changes: Vec<Change>,
}

impl ChangeLog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a committed change. Returns the change's id for ergonomic
    /// chaining.
    pub fn append(&mut self, change: Change) -> ChangeId {
        let id = change.id;
        self.changes.push(change);
        id
    }

    pub fn len(&self) -> usize {
        self.changes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Change> {
        self.changes.iter()
    }

    pub fn get(&self, id: ChangeId) -> Option<&Change> {
        self.changes.iter().find(|c| c.id == id)
    }

    /// Iterate the slice of changes that committed in a given batch.
    /// Linear scan for now — fine for small histories; an index lands
    /// when retention/weathering does.
    pub fn batch(&self, batch: BatchId) -> impl Iterator<Item = &Change> {
        self.changes.iter().filter(move |c| c.batch == batch)
    }

    /// Iterate the changes whose subject is a given locus, newest first.
    /// Used by the locus program when assembling its incoming inbox.
    pub fn changes_to_locus(&self, locus: LocusId) -> impl Iterator<Item = &Change> {
        self.changes.iter().rev().filter(move |c| match c.subject {
            ChangeSubject::Locus(id) => id == locus,
            ChangeSubject::Relationship(_) => false,
        })
    }

    /// Iterate the changes whose subject is a given relationship, newest
    /// first. Analogous to `changes_to_locus`.
    pub fn changes_to_relationship(
        &self,
        rel: graph_core::RelationshipId,
    ) -> impl Iterator<Item = &Change> {
        self.changes.iter().rev().filter(move |c| match c.subject {
            ChangeSubject::Relationship(id) => id == rel,
            ChangeSubject::Locus(_) => false,
        })
    }

    /// Remove all changes with a batch index strictly older than
    /// `retain_from_batch`.
    ///
    /// Returns the number of changes removed. This is a destructive,
    /// irreversible operation — callers should ensure that no live
    /// predecessor references point into the trimmed range. The engine
    /// enforces this by requiring callers to run `trim_change_log` only
    /// after `weather_entities` (which strips predecessor id lists from
    /// compressed layers) or in workloads where old predecessor ids are
    /// never queried.
    pub fn trim_before_batch(&mut self, retain_from_batch: BatchId) -> usize {
        let before = self.changes.len();
        self.changes.retain(|c| c.batch >= retain_from_batch);
        before - self.changes.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{InfluenceKindId, StateVector};

    fn change(id: u64, locus: u64, batch: u64) -> Change {
        Change {
            id: ChangeId(id),
            subject: ChangeSubject::Locus(LocusId(locus)),
            kind: InfluenceKindId(1),
            predecessors: Vec::new(),
            before: StateVector::empty(),
            after: StateVector::empty(),
            batch: BatchId(batch),
        }
    }

    #[test]
    fn batch_filter() {
        let mut log = ChangeLog::new();
        log.append(change(1, 10, 0));
        log.append(change(2, 11, 0));
        log.append(change(3, 10, 1));
        let in_batch_0: Vec<_> = log.batch(BatchId(0)).map(|c| c.id.0).collect();
        assert_eq!(in_batch_0, vec![1, 2]);
    }

    #[test]
    fn changes_to_locus_returns_newest_first() {
        let mut log = ChangeLog::new();
        log.append(change(1, 10, 0));
        log.append(change(2, 11, 0));
        log.append(change(3, 10, 1));
        let to_10: Vec<_> = log.changes_to_locus(LocusId(10)).map(|c| c.id.0).collect();
        assert_eq!(to_10, vec![3, 1]);
    }

    #[test]
    fn trim_before_batch_removes_older_entries() {
        let mut log = ChangeLog::new();
        log.append(change(1, 10, 0));
        log.append(change(2, 11, 0));
        log.append(change(3, 10, 1));
        log.append(change(4, 11, 2));

        let removed = log.trim_before_batch(BatchId(1));
        assert_eq!(removed, 2, "two entries in batch 0 removed");
        assert_eq!(log.len(), 2);
        let remaining_batches: Vec<u64> = log.iter().map(|c| c.batch.0).collect();
        assert!(remaining_batches.iter().all(|&b| b >= 1));
    }

    #[test]
    fn trim_before_batch_keeps_all_when_retain_is_zero() {
        let mut log = ChangeLog::new();
        log.append(change(1, 10, 0));
        log.append(change(2, 10, 5));
        let removed = log.trim_before_batch(BatchId(0));
        assert_eq!(removed, 0);
        assert_eq!(log.len(), 2);
    }
}
