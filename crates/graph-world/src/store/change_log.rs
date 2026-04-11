//! Append-only log of committed changes.
//!
//! The change log is the substrate's only history. Higher layers
//! (relationships, entities) derive their state from it. The log is the
//! "raw change log" memory layer in `docs/redesign.md` §3.5.
//! Use `trim_before_batch` to reclaim memory.
//!
//! ## Ordering invariant
//!
//! Changes are pushed in the order they commit, which the engine
//! guarantees is consistent with their causal partial order: if A is in
//! B's predecessor set, A is recorded earlier.
//!
//! ## ChangeId density invariant
//!
//! The engine assigns `ChangeId`s as a dense monotonically-increasing
//! sequence starting from 0 and appends each change immediately after
//! minting its id. As a result the log has no id gaps: the change at
//! index `i` always has `id = changes[0].id + i`. `get()` relies on
//! this to compute array indices in O(1). `trim_before_batch` shifts the
//! offset but preserves density.

use std::collections::VecDeque;

use graph_core::{BatchId, Change, ChangeId, ChangeSubject, LocusId, RelationshipId};
use rustc_hash::{FxHashMap, FxHashSet};

/// Append-only log with O(1) lookup by id and O(k) subject/batch-filtered iteration.
///
/// Three reverse indices are maintained on every `append` and `trim_before_batch`:
/// - `by_locus` / `by_relationship` — map each subject to the ordered list of
///   change ids that targeted it.
/// - `by_batch` — maps each `BatchId` to its change ids for O(k) batch queries.
#[derive(Debug, Default, Clone)]
pub struct ChangeLog {
    changes: Vec<Change>,
    by_locus: FxHashMap<LocusId, Vec<ChangeId>>,
    by_relationship: FxHashMap<RelationshipId, Vec<ChangeId>>,
    by_batch: FxHashMap<BatchId, Vec<ChangeId>>,
}

impl ChangeLog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a committed change. Returns the change's id for ergonomic
    /// chaining.
    pub fn append(&mut self, change: Change) -> ChangeId {
        let id = change.id;
        match change.subject {
            ChangeSubject::Locus(locus_id) => {
                self.by_locus.entry(locus_id).or_default().push(id);
            }
            ChangeSubject::Relationship(rel_id) => {
                self.by_relationship.entry(rel_id).or_default().push(id);
            }
        }
        self.by_batch.entry(change.batch).or_default().push(id);
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

    /// Look up a change by id in O(1).
    ///
    /// `ChangeId`s are assigned as a dense monotonically-increasing sequence
    /// and the log is append-only in causal order, so the id's position in
    /// the `Vec` is determined by subtracting the first entry's id (the
    /// offset shifts after `trim_before_batch`). Returns `None` for ids that
    /// were trimmed or not yet committed.
    pub fn get(&self, id: ChangeId) -> Option<&Change> {
        let offset = self.changes.first()?.id.0;
        let idx = id.0.checked_sub(offset)? as usize;
        let change = self.changes.get(idx)?;
        debug_assert_eq!(change.id, id, "ChangeId/Vec index invariant violated");
        Some(change)
    }

    /// Iterate the changes that committed in a given batch.
    /// O(k) where k is the number of changes in the batch.
    pub fn batch(&self, batch: BatchId) -> impl Iterator<Item = &Change> + '_ {
        self.by_batch
            .get(&batch)
            .into_iter()
            .flat_map(|ids| ids.iter())
            .filter_map(|&id| self.get(id))
    }

    /// Iterate the changes whose subject is a given locus, newest first.
    /// Used by the locus program when assembling its incoming inbox.
    /// O(k) where k is the number of changes to this locus.
    pub fn changes_to_locus(&self, locus: LocusId) -> impl Iterator<Item = &Change> + '_ {
        self.by_locus
            .get(&locus)
            .into_iter()
            .flat_map(|ids| ids.iter().rev())
            .filter_map(|&id| self.get(id))
    }

    /// Iterate the changes whose subject is a given relationship, newest
    /// first. Analogous to `changes_to_locus`.
    /// O(k) where k is the number of changes to this relationship.
    pub fn changes_to_relationship(&self, rel: RelationshipId) -> impl Iterator<Item = &Change> + '_ {
        self.by_relationship
            .get(&rel)
            .into_iter()
            .flat_map(|ids| ids.iter().rev())
            .filter_map(|&id| self.get(id))
    }

    /// Iterate the direct predecessor changes of `id` in predecessor-list
    /// order. Yields nothing if `id` is not in the log or has no predecessors.
    pub fn predecessors(&self, id: ChangeId) -> impl Iterator<Item = &Change> {
        self.get(id)
            .into_iter()
            .flat_map(|c| &c.predecessors)
            .filter_map(|pid| self.get(*pid))
    }

    /// Walk the causal DAG backwards from `start` in BFS order, returning
    /// every ancestor change. Each ancestor appears at most once. `start`
    /// itself is not included.
    ///
    /// Traversal stops at any predecessor whose id is no longer in the log
    /// (e.g. trimmed by `trim_before_batch`) — those ancestors are simply
    /// absent from the result.
    pub fn causal_ancestors(&self, start: ChangeId) -> Vec<&Change> {
        let Some(root) = self.get(start) else {
            return Vec::new();
        };
        let mut visited: FxHashSet<ChangeId> = FxHashSet::default();
        visited.insert(start);
        let mut queue: VecDeque<ChangeId> = root.predecessors.iter().copied().collect();
        let mut result = Vec::new();
        // Both guards must hold: if `get()` returns None the id was trimmed
        // and traversal stops for that branch (no panic, no queue extension).
        while let Some(id) = queue.pop_front() {
            if visited.insert(id) && let Some(change) = self.get(id) {
                result.push(change);
                queue.extend(change.predecessors.iter().copied());
            }
        }
        result
    }

    /// Return `true` if `ancestor` is a causal ancestor of `descendant`.
    ///
    /// Performs a DFS from `descendant` backwards through the predecessor DAG,
    /// pruning branches whose ids are already smaller than `ancestor.id`
    /// (predecessors always have smaller ids, so they can't lead to `ancestor`).
    /// Short-circuits as soon as `ancestor` is found.
    pub fn is_ancestor_of(&self, ancestor: ChangeId, descendant: ChangeId) -> bool {
        if ancestor.0 >= descendant.0 {
            return false;
        }
        let Some(desc) = self.get(descendant) else {
            return false;
        };
        let mut stack: Vec<ChangeId> = desc.predecessors
            .iter()
            .copied()
            .filter(|&pid| pid.0 >= ancestor.0)
            .collect();
        let mut visited: FxHashSet<ChangeId> = FxHashSet::default();
        while let Some(id) = stack.pop() {
            if id == ancestor {
                return true;
            }
            if visited.insert(id) && let Some(c) = self.get(id) {
                stack.extend(
                    c.predecessors.iter().copied().filter(|&pid| pid.0 >= ancestor.0),
                );
            }
        }
        false
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
        let split = self.changes.partition_point(|c| c.batch < retain_from_batch);
        if split == 0 {
            return 0;
        }
        if split == self.changes.len() {
            // Trimming the entire log — clear all indices.
            self.by_locus.clear();
            self.by_relationship.clear();
            self.by_batch.clear();
            self.changes.clear();
            return split;
        }
        // The first id we keep; id-indexed vecs are oldest-first so we can
        // use partition_point to strip the front of each vec in O(log k).
        let first_kept = self.changes[split].id;
        for ids in self.by_locus.values_mut() {
            let remove = ids.partition_point(|&id| id < first_kept);
            ids.drain(..remove);
        }
        for ids in self.by_relationship.values_mut() {
            let remove = ids.partition_point(|&id| id < first_kept);
            ids.drain(..remove);
        }
        // Batch entries for fully-trimmed batches are dropped wholesale.
        self.by_batch.retain(|&b, _| b >= retain_from_batch);
        self.changes.drain(..split).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{InfluenceKindId, StateVector};

    fn change_with_preds(id: u64, locus: u64, batch: u64, preds: Vec<u64>) -> Change {
        Change {
            id: ChangeId(id),
            subject: ChangeSubject::Locus(LocusId(locus)),
            kind: InfluenceKindId(1),
            predecessors: preds.into_iter().map(ChangeId).collect(),
            before: StateVector::empty(),
            after: StateVector::empty(),
            batch: BatchId(batch),
            wall_time: None,
            metadata: None,
        }
    }

    fn change(id: u64, locus: u64, batch: u64) -> Change {
        change_with_preds(id, locus, batch, vec![])
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

    #[test]
    fn predecessors_returns_direct_parents() {
        let mut log = ChangeLog::new();
        log.append(change(1, 10, 0));
        log.append(change(2, 11, 0));
        log.append(change_with_preds(3, 10, 1, vec![1, 2]));

        let preds: Vec<u64> = log.predecessors(ChangeId(3)).map(|c| c.id.0).collect();
        assert_eq!(preds, vec![1, 2]);
    }

    #[test]
    fn predecessors_of_stimulus_is_empty() {
        let mut log = ChangeLog::new();
        log.append(change(1, 10, 0));
        assert_eq!(log.predecessors(ChangeId(1)).count(), 0);
    }

    #[test]
    fn causal_ancestors_walks_full_dag() {
        // DAG: 3 → {1, 2}, 4 → {3}, so ancestors of 4 are {3, 1, 2}
        let mut log = ChangeLog::new();
        log.append(change(1, 10, 0));
        log.append(change(2, 11, 0));
        log.append(change_with_preds(3, 10, 1, vec![1, 2]));
        log.append(change_with_preds(4, 10, 2, vec![3]));

        let mut ancestors: Vec<u64> = log.causal_ancestors(ChangeId(4)).iter().map(|c| c.id.0).collect();
        ancestors.sort_unstable();
        assert_eq!(ancestors, vec![1, 2, 3]);
    }

    #[test]
    fn is_ancestor_of_direct_predecessor() {
        let mut log = ChangeLog::new();
        log.append(change(1, 10, 0));
        log.append(change_with_preds(2, 11, 1, vec![1]));
        assert!(log.is_ancestor_of(ChangeId(1), ChangeId(2)));
        assert!(!log.is_ancestor_of(ChangeId(2), ChangeId(1)));
    }

    #[test]
    fn is_ancestor_of_transitive() {
        // 1 → 2 → 3; is_ancestor_of(1, 3) == true
        let mut log = ChangeLog::new();
        log.append(change(1, 10, 0));
        log.append(change_with_preds(2, 11, 1, vec![1]));
        log.append(change_with_preds(3, 12, 2, vec![2]));
        assert!(log.is_ancestor_of(ChangeId(1), ChangeId(3)));
        assert!(!log.is_ancestor_of(ChangeId(3), ChangeId(1)));
    }

    #[test]
    fn is_ancestor_of_unrelated() {
        let mut log = ChangeLog::new();
        log.append(change(1, 10, 0));
        log.append(change(2, 11, 0));
        assert!(!log.is_ancestor_of(ChangeId(1), ChangeId(2)));
        assert!(!log.is_ancestor_of(ChangeId(2), ChangeId(1)));
    }

    #[test]
    fn causal_ancestors_deduplicates_shared_ancestors() {
        // DAG: 1 → {0}, 2 → {0} (diamond), 3 → {1, 2}; ancestor 0 appears once
        let mut log = ChangeLog::new();
        log.append(change(0, 10, 0));
        log.append(change_with_preds(1, 11, 1, vec![0]));
        log.append(change_with_preds(2, 12, 1, vec![0]));
        log.append(change_with_preds(3, 10, 2, vec![1, 2]));

        let mut ancestors: Vec<u64> = log.causal_ancestors(ChangeId(3)).iter().map(|c| c.id.0).collect();
        ancestors.sort_unstable();
        assert_eq!(ancestors, vec![0, 1, 2]);
    }
}
