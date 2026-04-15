//! Temporal and batch-relative query helpers.
//!
//! Provides relative-batch convenience wrappers, per-batch statistics, and
//! change-frequency analysis over the `ChangeLog`.
//!
//! ## Examples
//!
//! ```ignore
//! // What happened in batch 42?
//! let stats = graph_query::batch_stats(&world, BatchId(42));
//!
//! // Which loci were most active in the last 10 batches?
//! let current = world.current_batch().0;
//! let hot = graph_query::loci_by_change_frequency(
//!     &world,
//!     BatchId(current.saturating_sub(10)),
//!     world.current_batch(),
//! );
//! ```

use graph_core::{BatchId, Change, ChangeSubject, LocusId, RelationshipId};
use graph_world::World;
use rustc_hash::{FxHashMap, FxHashSet};

// ─── BatchStats ───────────────────────────────────────────────────────────────

/// Aggregate statistics for a single committed batch.
///
/// Produced by [`batch_stats`].
#[derive(Debug, Clone, PartialEq)]
pub struct BatchStats {
    /// The batch these stats describe.
    pub batch: BatchId,
    /// Total number of committed changes (including relationship-subject ones).
    pub total_changes: usize,
    /// Number of distinct loci that had at least one change committed.
    pub loci_changed: usize,
    /// Number of distinct relationships that had at least one explicit change.
    pub relationships_changed: usize,
    /// Mean per-slot L1 magnitude of state transitions across all changes.
    ///
    /// Computed as the mean of `|after[i] - before[i]|` over every slot of
    /// every change in the batch. A proxy for "how much state moved this batch".
    pub mean_delta: f32,
}

/// Compute aggregate statistics for `batch`, or `None` if the batch has no
/// committed changes (e.g. the batch hasn't run yet or was fully trimmed).
pub fn batch_stats(world: &World, batch: BatchId) -> Option<BatchStats> {
    let changes: Vec<&Change> = world.log().batch(batch).collect();
    if changes.is_empty() {
        return None;
    }

    let mut loci: FxHashSet<LocusId> = FxHashSet::default();
    let mut rels: FxHashSet<RelationshipId> = FxHashSet::default();
    let mut total_delta = 0.0f32;
    let mut delta_count = 0usize;

    for c in &changes {
        match c.subject {
            ChangeSubject::Locus(id) => {
                loci.insert(id);
            }
            ChangeSubject::Relationship(id) => {
                rels.insert(id);
            }
        }
        let before = c.before.as_slice();
        let after = c.after.as_slice();
        let len = before.len().max(after.len());
        for i in 0..len {
            let b = before.get(i).copied().unwrap_or(0.0);
            let a = after.get(i).copied().unwrap_or(0.0);
            total_delta += (a - b).abs();
            delta_count += 1;
        }
    }

    Some(BatchStats {
        batch,
        total_changes: changes.len(),
        loci_changed: loci.len(),
        relationships_changed: rels.len(),
        mean_delta: if delta_count > 0 {
            total_delta / delta_count as f32
        } else {
            0.0
        },
    })
}

// ─── Recent-change convenience ────────────────────────────────────────────────

/// The last `n` changes committed to `locus`, newest first.
///
/// More ergonomic than `changes_to_locus_in_range` when you only want the most
/// recent N entries and don't know the batch range in advance.
pub fn last_n_changes_to_locus(world: &World, locus: LocusId, n: usize) -> Vec<&Change> {
    world.changes_to_locus(locus).take(n).collect()
}

/// The last `n` changes committed to `rel`, newest first.
///
/// Only `ChangeSubject::Relationship` entries are returned — auto-emerge
/// touches are not recorded as relationship-subject changes.
pub fn last_n_changes_to_relationship(
    world: &World,
    rel: RelationshipId,
    n: usize,
) -> Vec<&Change> {
    world.changes_to_relationship(rel).take(n).collect()
}

// ─── Change-frequency analysis ────────────────────────────────────────────────

/// All loci that had at least one change committed at or after `since_batch`,
/// sorted by change count descending (most-active first).
///
/// Returns `(LocusId, change_count)` pairs. Useful for spotting "hot spots"
/// that became active after a reference point (e.g. after a major input event).
pub fn changed_since(world: &World, since_batch: BatchId) -> Vec<(LocusId, usize)> {
    let mut counts: FxHashMap<LocusId, usize> = FxHashMap::default();
    for c in world.log().iter() {
        if c.batch >= since_batch && let ChangeSubject::Locus(id) = c.subject {
            *counts.entry(id).or_insert(0) += 1;
        }
    }
    let mut result: Vec<_> = counts.into_iter().collect();
    result.sort_by(|a, b| b.1.cmp(&a.1));
    result
}

/// Loci ranked by total change count in `[from_batch, to_batch]`, most-changed
/// first.
///
/// Returns `(LocusId, change_count)` pairs. Loci with zero changes in the range
/// are not included.
pub fn loci_by_change_frequency(
    world: &World,
    from_batch: BatchId,
    to_batch: BatchId,
) -> Vec<(LocusId, usize)> {
    let mut counts: FxHashMap<LocusId, usize> = FxHashMap::default();
    for c in world.log().iter() {
        if c.batch >= from_batch && c.batch <= to_batch && let ChangeSubject::Locus(id) = c.subject {
            *counts.entry(id).or_insert(0) += 1;
        }
    }
    let mut result: Vec<_> = counts.into_iter().collect();
    result.sort_by(|a, b| b.1.cmp(&a.1));
    result
}

/// Relationships ranked by total explicit change count in `[from_batch,
/// to_batch]`, most-changed first.
///
/// Returns `(RelationshipId, change_count)` pairs. Only
/// `ChangeSubject::Relationship` entries are counted — auto-emerged touches
/// (which are not logged as relationship-subject changes) are not included here.
/// For a count that includes auto-emerge touches, use
/// `world.relationships().get(id)?.lineage.change_count`.
pub fn relationships_by_change_frequency(
    world: &World,
    from_batch: BatchId,
    to_batch: BatchId,
) -> Vec<(RelationshipId, usize)> {
    let mut counts: FxHashMap<RelationshipId, usize> = FxHashMap::default();
    for c in world.log().iter() {
        if c.batch >= from_batch && c.batch <= to_batch && let ChangeSubject::Relationship(id) = c.subject {
            *counts.entry(id).or_insert(0) += 1;
        }
    }
    let mut result: Vec<_> = counts.into_iter().collect();
    result.sort_by(|a, b| b.1.cmp(&a.1));
    result
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{Change, ChangeId, ChangeSubject, InfluenceKindId, LocusId, StateVector};
    use graph_world::World;

    fn push_locus_change(world: &mut World, id: u64, locus: u64, batch: u64, before: f32, after: f32) {
        let cid = ChangeId(id);
        world.log_mut().append(Change {
            id: cid,
            subject: ChangeSubject::Locus(LocusId(locus)),
            kind: InfluenceKindId(1),
            predecessors: vec![],
            before: StateVector::from_slice(&[before]),
            after: StateVector::from_slice(&[after]),
            batch: BatchId(batch),
            wall_time: None,
            metadata: None,
        });
    }

    fn push_rel_change(world: &mut World, id: u64, rel: u64, batch: u64) {
        let cid = ChangeId(id);
        world.log_mut().append(Change {
            id: cid,
            subject: ChangeSubject::Relationship(RelationshipId(rel)),
            kind: InfluenceKindId(1),
            predecessors: vec![],
            before: StateVector::zeros(2),
            after: StateVector::from_slice(&[0.5, 0.0]),
            batch: BatchId(batch),
            wall_time: None,
            metadata: None,
        });
    }

    #[test]
    fn batch_stats_none_for_empty_batch() {
        let w = World::new();
        assert!(batch_stats(&w, BatchId(99)).is_none());
    }

    #[test]
    fn batch_stats_counts_correctly() {
        let mut w = World::new();
        push_locus_change(&mut w, 0, 0, 5, 0.0, 0.5);
        push_locus_change(&mut w, 1, 1, 5, 0.2, 0.8);
        push_rel_change(&mut w, 2, 10, 5);

        let stats = batch_stats(&w, BatchId(5)).unwrap();
        assert_eq!(stats.batch, BatchId(5));
        assert_eq!(stats.total_changes, 3);
        assert_eq!(stats.loci_changed, 2);
        assert_eq!(stats.relationships_changed, 1);
        // mean delta: changes 0 and 1 each have 1 slot, deltas = 0.5 and 0.6
        // change 2 has 2 slots, deltas = 0.5 and 0.0
        // total_delta = 0.5 + 0.6 + 0.5 + 0.0 = 1.6; delta_count = 4
        let expected_mean = 1.6 / 4.0;
        assert!((stats.mean_delta - expected_mean).abs() < 1e-5);
    }

    #[test]
    fn last_n_changes_to_locus_limits_count() {
        let mut w = World::new();
        push_locus_change(&mut w, 0, 0, 1, 0.0, 0.1);
        push_locus_change(&mut w, 1, 0, 2, 0.1, 0.2);
        push_locus_change(&mut w, 2, 0, 3, 0.2, 0.3);

        let last2 = last_n_changes_to_locus(&w, LocusId(0), 2);
        assert_eq!(last2.len(), 2);
        // Newest first: batch 3, then batch 2.
        assert_eq!(last2[0].batch, BatchId(3));
        assert_eq!(last2[1].batch, BatchId(2));
    }

    #[test]
    fn last_n_changes_returns_all_when_fewer_than_n() {
        let mut w = World::new();
        push_locus_change(&mut w, 0, 5, 1, 0.0, 0.5);

        let result = last_n_changes_to_locus(&w, LocusId(5), 10);
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn changed_since_returns_sorted_by_count() {
        let mut w = World::new();
        // Locus 0: 3 changes at batches 2, 3, 4
        // Locus 1: 1 change at batch 3
        // Locus 2: 2 changes at batch 1 (before since_batch=2 — should not appear)
        push_locus_change(&mut w, 0, 0, 2, 0.0, 0.1);
        push_locus_change(&mut w, 1, 0, 3, 0.1, 0.2);
        push_locus_change(&mut w, 2, 0, 4, 0.2, 0.3);
        push_locus_change(&mut w, 3, 1, 3, 0.0, 0.5);
        push_locus_change(&mut w, 4, 2, 1, 0.0, 0.9);
        push_locus_change(&mut w, 5, 2, 1, 0.9, 0.8);

        let result = changed_since(&w, BatchId(2));
        // Only loci 0 (3×) and 1 (1×) are in range.
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].0, LocusId(0));
        assert_eq!(result[0].1, 3);
        assert_eq!(result[1].0, LocusId(1));
        assert_eq!(result[1].1, 1);
    }

    #[test]
    fn loci_by_change_frequency_uses_inclusive_range() {
        let mut w = World::new();
        push_locus_change(&mut w, 0, 0, 1, 0.0, 0.1); // outside
        push_locus_change(&mut w, 1, 0, 2, 0.1, 0.2); // inside
        push_locus_change(&mut w, 2, 0, 3, 0.2, 0.3); // inside
        push_locus_change(&mut w, 3, 0, 4, 0.3, 0.4); // outside
        push_locus_change(&mut w, 4, 1, 2, 0.0, 0.5); // inside

        let result = loci_by_change_frequency(&w, BatchId(2), BatchId(3));
        // Locus 0: 2, locus 1: 1 — sorted desc
        assert_eq!(result[0], (LocusId(0), 2));
        assert_eq!(result[1], (LocusId(1), 1));
    }

    #[test]
    fn relationships_by_change_frequency_counts_only_rel_subjects() {
        let mut w = World::new();
        push_locus_change(&mut w, 0, 0, 1, 0.0, 0.5); // locus — not counted
        push_rel_change(&mut w, 1, 10, 1);
        push_rel_change(&mut w, 2, 10, 1);
        push_rel_change(&mut w, 3, 20, 1);

        let result = relationships_by_change_frequency(&w, BatchId(1), BatchId(1));
        // rel 10: 2, rel 20: 1
        assert_eq!(result[0].0, RelationshipId(10));
        assert_eq!(result[0].1, 2);
        assert_eq!(result[1].0, RelationshipId(20));
        assert_eq!(result[1].1, 1);
    }
}
