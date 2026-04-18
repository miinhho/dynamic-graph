//! Causal DAG queries over the change log.
//!
//! The `ChangeLog` records causal predecessor edges for every committed
//! change. These functions let callers walk that graph: find all ancestors
//! of a change, or the complete set of changes that affected a locus within
//! a batch range.
//!
//! All queries are read-only over `&World`.

use graph_core::{
    BatchId, Change, ChangeId, ChangeSubject, LocusId, Relationship, RelationshipId, TrimSummary,
};
use graph_world::World;
use rustc_hash::{FxHashMap, FxHashSet};

// ─── Latest-change convenience ───────────────────────────────────────────────

/// The most recent change committed to `locus`, or `None` if the locus has
/// never been changed or its history has been fully trimmed.
///
/// Equivalent to `world.changes_to_locus(locus).next()` but more discoverable
/// as a named function and avoids the iterator import.
pub fn last_change_to_locus(world: &World, locus: LocusId) -> Option<&Change> {
    world.changes_to_locus(locus).next()
}

/// The most recent change committed to `rel`, or `None` if the relationship
/// has no explicit change history or it has been fully trimmed.
///
/// Only `ChangeSubject::Relationship` changes are considered (subscription
/// observer changes). Auto-emergence touches are not recorded as
/// relationship-subject changes.
pub fn last_change_to_relationship(world: &World, rel: RelationshipId) -> Option<&Change> {
    world.changes_to_relationship(rel).next()
}

// ─── Committed batch discovery ───────────────────────────────────────────────

/// All batch IDs that have at least one committed change, in ascending order.
///
/// Wraps `ChangeLog::committed_batch_ids`. Use this to iterate the full commit
/// history without needing to know the exact batch range in advance:
///
/// ```ignore
/// for batch in graph_query::committed_batches(&world) {
///     let changed = graph_query::loci_changed_in_batch(&world, batch);
///     // …
/// }
/// ```
///
/// After `trim_before_batch`, only batches at or after the retain boundary
/// are returned.
pub fn committed_batches(world: &World) -> Vec<BatchId> {
    world.log().committed_batch_ids()
}

// ─── Batch-temporal queries ───────────────────────────────────────────────────

/// All loci that had at least one change committed in `batch`.
///
/// Uses the `ChangeLog::batch` reverse index — O(k) where k is the number of
/// changes in that batch. Deduplicates: each locus appears at most once even
/// if it had multiple changes in the batch.
pub fn loci_changed_in_batch(world: &World, batch: BatchId) -> Vec<LocusId> {
    let mut seen = FxHashSet::default();
    world
        .log()
        .batch(batch)
        .filter_map(|c| match c.subject {
            ChangeSubject::Locus(id) => {
                if seen.insert(id) {
                    Some(id)
                } else {
                    None
                }
            }
            ChangeSubject::Relationship(_) => None,
        })
        .collect()
}

/// All relationships that had at least one explicit change committed in `batch`.
///
/// Only changes with `ChangeSubject::Relationship` are considered — changes
/// that merely touched a relationship via locus auto-emergence are not
/// recorded as relationship-subject changes and will not appear here.
///
/// Deduplicates: each relationship appears at most once.
pub fn relationships_changed_in_batch(world: &World, batch: BatchId) -> Vec<RelationshipId> {
    let mut seen = FxHashSet::default();
    world
        .log()
        .batch(batch)
        .filter_map(|c| match c.subject {
            ChangeSubject::Relationship(id) => {
                if seen.insert(id) {
                    Some(id)
                } else {
                    None
                }
            }
            ChangeSubject::Locus(_) => None,
        })
        .collect()
}

// ─── Common ancestors ─────────────────────────────────────────────────────────

/// The set of changes that are causal ancestors of **both** `a` and `b`.
///
/// Computed as the intersection of the two ancestor BFS walks. Useful for
/// identifying the shared causal context of two independent downstream changes.
///
/// Returns an empty `Vec` when either change has no ancestors, or when their
/// ancestor sets are disjoint.
pub fn common_ancestors(world: &World, a: ChangeId, b: ChangeId) -> Vec<ChangeId> {
    let ancestors_a: FxHashSet<ChangeId> = causal_ancestors(world, a).into_iter().collect();
    causal_ancestors(world, b)
        .into_iter()
        .filter(|id| ancestors_a.contains(id))
        .collect()
}

// ─── Causal depth ─────────────────────────────────────────────────────────────

/// The depth of `change_id` in the causal DAG — the length of the longest
/// predecessor chain leading back to any root stimulus.
///
/// - Depth 0: `change_id` is a root (no predecessors, or not in the log).
/// - Depth N: there exists a predecessor chain of length N reaching a root.
///
/// Uses iterative post-order DFS with memoisation to avoid stack overflow on
/// long causal chains. Stops at changes trimmed from the log (treated as
/// depth-0 roots from the trimmed boundary).
pub fn causal_depth(world: &World, change_id: ChangeId) -> usize {
    use rustc_hash::FxHashMap;
    let mut memo: FxHashMap<ChangeId, usize> = FxHashMap::default();
    // Stack entries: (change_id, processed). First visit pushes predecessors;
    // second visit (processed=true) computes and memos the depth.
    let mut stack: Vec<(ChangeId, bool)> = vec![(change_id, false)];

    while let Some((cid, processed)) = stack.pop() {
        if processed {
            let depth = world
                .log()
                .get(cid)
                .map(|c| {
                    if c.predecessors.is_empty() {
                        0
                    } else {
                        c.predecessors
                            .iter()
                            .map(|&p| memo.get(&p).copied().unwrap_or(0) + 1)
                            .max()
                            .unwrap_or(0)
                    }
                })
                .unwrap_or(0);
            memo.insert(cid, depth);
        } else if !memo.contains_key(&cid) {
            stack.push((cid, true));
            if let Some(c) = world.log().get(cid) {
                for &p in &c.predecessors {
                    if !memo.contains_key(&p) {
                        stack.push((p, false));
                    }
                }
            }
        }
    }
    memo.get(&change_id).copied().unwrap_or(0)
}

// ─── Relationship causality ───────────────────────────────────────────────────

/// Walk backwards from the creation of `rel` to find all root changes
/// (stimuli — changes with no predecessors) that ultimately caused the
/// relationship to form.
///
/// Returns an empty `Vec` when:
/// - The relationship is not in the world.
/// - The relationship was pre-created with no `created_by` change (e.g.
///   inserted before the engine ran, or via `StructuralProposal::CreateRelationship`).
///
/// Returns `vec![created_by]` when the creation change itself is a root
/// stimulus (no predecessors). Otherwise walks the DAG from `created_by`
/// and returns all leaf ancestors.
///
/// This is the primary API for answering **"why does this relationship exist?"**
pub fn root_stimuli_for_relationship(world: &World, rel: RelationshipId) -> Vec<ChangeId> {
    let Some(created_by) = world
        .relationships()
        .get(rel)
        .and_then(|r| r.lineage.created_by)
    else {
        return Vec::new();
    };

    // If the creation change is itself a root stimulus, return it directly.
    if world
        .log()
        .get(created_by)
        .is_some_and(|c| c.predecessors.is_empty())
    {
        return vec![created_by];
    }

    root_stimuli(world, created_by)
}

// ─── Relationship volatility ──────────────────────────────────────────────────

/// Activity volatility of `rel` over `[from_batch, to_batch]`.
///
/// Computed as the **standard deviation** of the activity slot (slot 0) across
/// all explicit `ChangeSubject::Relationship` changes in the range. A value
/// close to 0.0 indicates stable, steady coupling; a high value indicates
/// burst-driven or oscillating coupling.
///
/// ## When this returns 0.0
///
/// This function measures volatility via the `ChangeLog`. Relationship log
/// entries (`ChangeSubject::Relationship`) are only created when a program
/// explicitly proposes a relationship-subject change — typically through a
/// **subscription** (`SubscribeToRelationship`). Auto-emerged relationships
/// that are touched exclusively through locus cross-coupling have **no**
/// relationship-subject log entries, so this always returns `0.0` for them.
///
/// For auto-emerged relationships, use
/// [`relationship_touch_rate`][crate::relationship_touch_rate] instead, which
/// derives its metric from `lineage.change_count` and `created_batch` — both
/// present on every relationship regardless of how it was created.
pub fn relationship_volatility(
    world: &World,
    rel: RelationshipId,
    from_batch: BatchId,
    to_batch: BatchId,
) -> f32 {
    let changes = changes_to_relationship_in_range(world, rel, from_batch, to_batch);
    let n = changes.len();
    if n < 2 {
        return 0.0;
    }
    let nf = n as f32;
    let activity = |c: &&Change| c.after.as_slice().first().copied().unwrap_or(0.0);
    let mean = changes.iter().map(activity).sum::<f32>() / nf;
    let variance = changes
        .iter()
        .map(|c| (activity(c) - mean).powi(2))
        .sum::<f32>()
        / nf;
    variance.sqrt()
}

/// Activity volatility of `rel` over its **entire committed history**.
///
/// Convenience wrapper around `relationship_volatility` that automatically
/// uses `BatchId(0)` as the start and the world's current batch as the end.
/// See [`relationship_volatility`] for the definition and the note on when this
/// returns `0.0` (auto-emerged relationships without subscriptions).
pub fn relationship_volatility_all(world: &World, rel: RelationshipId) -> f32 {
    relationship_volatility(world, rel, BatchId(0), world.current_batch())
}

// ─── Activity trend ───────────────────────────────────────────────────────────

/// Directional trend of a relationship's activity over explicit change history.
///
/// Computed via ordinary least-squares linear regression on the sequence of
/// `after[0]` (activity slot) values recorded in `ChangeSubject::Relationship`
/// log entries within `[from_batch, to_batch]`.
///
/// # Variants
///
/// - `Rising { slope }` — activity is increasing. `slope > 0`.
/// - `Falling { slope }` — activity is decreasing. `slope < 0`.
/// - `Stable` — slope is within `±threshold` (default `0.05`).
///
/// # Limitation
///
/// Like `relationship_volatility`, this query relies on the `ChangeLog`.
/// Auto-emerged relationships that are never the subject of an explicit
/// program-proposed change have **no** log entries and will return `None`.
/// For those, use `relationship_touch_rate` or examine the current activity
/// directly via `world.relationships().get(rel_id)?.activity()`.
#[derive(Debug, Clone, PartialEq)]
pub enum Trend {
    /// Activity is growing over the window. `slope` is the regression
    /// coefficient in activity-units-per-batch-index (always positive).
    Rising { slope: f32 },
    /// Activity is shrinking over the window. `slope` is always negative.
    Falling { slope: f32 },
    /// No statistically meaningful direction within `±threshold`.
    Stable,
}

/// Compute the activity trend of `rel` over `[from_batch, to_batch]`.
///
/// Returns `None` when there are fewer than two change-log entries in the
/// range (insufficient data for regression). See [`Trend`] for the definition
/// of each variant and the note on auto-emerged relationships.
pub fn relationship_activity_trend(
    world: &World,
    rel: RelationshipId,
    from_batch: BatchId,
    to_batch: BatchId,
) -> Option<Trend> {
    relationship_activity_trend_with_threshold(world, rel, from_batch, to_batch, 0.05)
}

/// Like [`relationship_activity_trend`] but with an explicit `stable_threshold`.
///
/// A slope whose absolute value is ≤ `stable_threshold` is classified as
/// `Trend::Stable`. Default in the non-threshold variant is `0.05`.
pub fn relationship_activity_trend_with_threshold(
    world: &World,
    rel: RelationshipId,
    from_batch: BatchId,
    to_batch: BatchId,
    stable_threshold: f32,
) -> Option<Trend> {
    let changes = changes_to_relationship_in_range(world, rel, from_batch, to_batch);
    let n = changes.len();
    if n < 2 {
        return None;
    }

    let Some(slope) = ols_activity_slope(&changes) else {
        return Some(Trend::Stable);
    };

    Some(if slope > stable_threshold {
        Trend::Rising { slope }
    } else if slope < -stable_threshold {
        Trend::Falling { slope }
    } else {
        Trend::Stable
    })
}

// ─── Weight trend ─────────────────────────────────────────────────────────────

/// Directional trend of a relationship's **weight** (slot 1) over explicit
/// Hebbian/STDP change history.
///
/// Hebbian plasticity updates emit `ChangeSubject::Relationship` entries whose
/// `after` state carries the full relationship `StateVector` (`[activity,
/// weight, …]`). This function reads slot 1 (the weight slot) from those
/// entries to compute the OLS slope.
///
/// Returns `None` when there are fewer than two Hebbian log entries in the
/// range (insufficient data for regression). Auto-emerged relationships that
/// were not subject to any Hebbian updates will return `None`.
///
/// # Contrast with `relationship_activity_trend`
///
/// `relationship_activity_trend` reads `after[0]` (the activity slot) and is
/// intended for subscription-observer changes. This function reads `after[1]`
/// (the weight slot) and is intended for Hebbian plasticity changes.
pub fn relationship_weight_trend(
    world: &World,
    rel: RelationshipId,
    from_batch: BatchId,
    to_batch: BatchId,
) -> Option<Trend> {
    relationship_weight_trend_with_threshold(world, rel, from_batch, to_batch, 0.05)
}

/// Net weight change of `rel` over `[from_batch, to_batch]`.
///
/// Computes `newest_weight − oldest_weight` using the first and last
/// `ChangeSubject::Relationship` entries in the range (slot 1 = weight slot).
/// Unlike `relationship_weight_trend` (which uses OLS), this directly measures
/// the **total accumulated change** — useful for signals that saturate quickly
/// and would register a near-zero OLS slope even after significant net growth.
///
/// Returns `None` when there are fewer than two ChangeLog entries in the range.
pub fn relationship_weight_delta(
    world: &World,
    rel: RelationshipId,
    from_batch: BatchId,
    to_batch: BatchId,
) -> Option<f32> {
    let changes = changes_to_relationship_in_range(world, rel, from_batch, to_batch);
    if changes.len() < 2 {
        return None;
    }
    let newest = changes
        .first()?
        .after
        .as_slice()
        .get(Relationship::WEIGHT_SLOT)
        .copied()?;
    let oldest = changes
        .last()?
        .after
        .as_slice()
        .get(Relationship::WEIGHT_SLOT)
        .copied()?;
    Some(newest - oldest)
}

/// Classify the weight trend of `rel` using first-vs-last entry comparison.
///
/// Unlike `relationship_weight_trend` (OLS regression), this is robust to
/// **fast-saturating** Hebbian signals that plateau after a few ticks and
/// would register a near-zero OLS slope despite a large net weight gain.
///
/// Returns `None` when there are fewer than two ChangeLog entries in the
/// range. Returns `Trend::Stable` when the delta is within `±stable_threshold`.
pub fn relationship_weight_trend_delta(
    world: &World,
    rel: RelationshipId,
    from_batch: BatchId,
    to_batch: BatchId,
    stable_threshold: f32,
) -> Option<Trend> {
    let delta = relationship_weight_delta(world, rel, from_batch, to_batch)?;
    Some(if delta > stable_threshold {
        Trend::Rising { slope: delta }
    } else if delta < -stable_threshold {
        Trend::Falling { slope: delta }
    } else {
        Trend::Stable
    })
}

/// Like [`relationship_weight_trend`] but with an explicit `stable_threshold`.
pub fn relationship_weight_trend_with_threshold(
    world: &World,
    rel: RelationshipId,
    from_batch: BatchId,
    to_batch: BatchId,
    stable_threshold: f32,
) -> Option<Trend> {
    let changes = changes_to_relationship_in_range(world, rel, from_batch, to_batch);
    let n = changes.len();
    if n < 2 {
        return None;
    }

    let Some(slope) = ols_slot_slope(&changes, 1) else {
        return Some(Trend::Stable);
    };

    Some(if slope > stable_threshold {
        Trend::Rising { slope }
    } else if slope < -stable_threshold {
        Trend::Falling { slope }
    } else {
        Trend::Stable
    })
}

// ─── Coarse trail (E2) ───────────────────────────────────────────────────────

/// Result of a causal walk that crossed a trim boundary.
///
/// `causal_coarse_trail` walks the predecessor DAG as far as the log allows
/// (populating `fine`), then collects `TrimSummary` entries for every locus
/// whose causal chain was cut by `trim_before_batch` (populating `coarse`).
///
/// This lets callers answer "why did X happen?" even when the raw early history
/// was discarded — the coarse trail gives aggregate counts, net state deltas,
/// and the influence kinds that were active in the trimmed window.
#[derive(Debug, Clone, PartialEq)]
pub struct CoarseTrail {
    /// Fine-grained ancestor `ChangeId`s still present in the log.
    pub fine: Vec<ChangeId>,
    /// Coarse summaries from `ChangeLog::trim_summaries_for_locus` for every
    /// locus whose causal predecessor was trimmed away.
    pub coarse: Vec<TrimSummary>,
}

impl CoarseTrail {
    /// `true` when no fine ancestors and no coarse summaries — the target has
    /// no causal context at all (root stimulus or not in the log).
    pub fn is_empty(&self) -> bool {
        self.fine.is_empty() && self.coarse.is_empty()
    }

    /// `true` when the coarse trail is complete — no coarse summaries were
    /// needed (causal history is fully available in the log).
    pub fn is_exact(&self) -> bool {
        self.coarse.is_empty()
    }
}

/// Walk the predecessor DAG from `target`, returning both fine-grained
/// ancestors still in the log and coarse summaries for any trimmed ranges
/// that were crossed.
///
/// **Algorithm:**
/// 1. Run the normal BFS ancestor walk (collecting `fine`).
/// 2. For each change in `fine` (and `target` itself) that has a predecessor
///    ID below the log's earliest retained ID, record the change's subject locus.
/// 3. Fetch `trim_summaries_for_locus` for each such locus and flatten into
///    `coarse`.
///
/// Deduplicates coarse summaries — each locus contributes at most one
/// contiguous batch-range summary per `trim_before_batch` call.
///
/// Returns an empty `CoarseTrail` when `target` is not in the log.
pub fn causal_coarse_trail(world: &World, target: ChangeId) -> CoarseTrail {
    let log = world.log();

    // The first retained ID; anything strictly below was trimmed.
    let first_retained = log.iter().next().map(|c| c.id.0).unwrap_or(u64::MAX);

    let fine_changes = log.causal_ancestors(target);
    let fine: Vec<ChangeId> = fine_changes.iter().map(|c| c.id).collect();

    // Collect loci whose changes were trimmed and are referenced as predecessors.
    // Use `trimmed_locus_for` to resolve each trimmed predecessor ID back to its locus.
    let mut trimmed_loci: FxHashMap<LocusId, ()> = FxHashMap::default();

    let mut check_preds = |preds: &[ChangeId]| {
        for &pred in preds {
            if pred.0 < first_retained {
                if let Some(lid) = log.trimmed_locus_for(pred) {
                    trimmed_loci.insert(lid, ());
                }
            }
        }
    };

    if let Some(root) = log.get(target) {
        check_preds(&root.predecessors);
    }
    for c in &fine_changes {
        check_preds(&c.predecessors);
    }

    let coarse: Vec<TrimSummary> = trimmed_loci
        .keys()
        .flat_map(|&lid| log.trim_summaries_for_locus(lid))
        .cloned()
        .collect();

    CoarseTrail { fine, coarse }
}

// ─── OLS helper ──────────────────────────────────────────────────────────────

/// OLS linear regression slope over a sequence of `Change` values for a
/// given `StateVector` slot index.
///
/// x = change index (0..n-1), y = `change.after[slot_idx]`.
///
/// Returns `None` when the x-variance is near-zero (constant x, or n < 2),
/// or when `slot_idx` is out of bounds for the changes.
pub(crate) fn ols_slot_slope(changes: &[&Change], slot_idx: usize) -> Option<f32> {
    let n = changes.len();
    if n < 2 {
        return None;
    }
    let nf = n as f32;
    let sum_x = nf * (nf - 1.0) / 2.0;
    let sum_x2 = nf * (nf - 1.0) * (2.0 * nf - 1.0) / 6.0;
    let (sum_y, sum_xy) = changes
        .iter()
        .enumerate()
        .fold((0.0f32, 0.0f32), |(sy, sxy), (i, c)| {
            let a = c.after.as_slice().get(slot_idx).copied().unwrap_or(0.0);
            (sy + a, sxy + i as f32 * a)
        });
    let denom = nf * sum_x2 - sum_x * sum_x;
    if denom.abs() < 1e-12 {
        return None;
    }
    Some((nf * sum_xy - sum_x * sum_y) / denom)
}

/// OLS linear regression slope over a sequence of `Change` activity values.
///
/// x = change index (0..n-1), y = `change.after[0]` (activity slot).
///
/// Returns `None` when the x-variance is near-zero (constant x, or n < 2).
/// Used by both `relationship_activity_trend_with_threshold` and
/// `profile::ols_slope_for_rel` to avoid duplicating the formula.
pub(crate) fn ols_activity_slope(changes: &[&Change]) -> Option<f32> {
    ols_slot_slope(changes, 0)
}

// ─── Ancestor queries ─────────────────────────────────────────────────────────

/// All causal ancestor `ChangeId`s of `target`, via BFS in the predecessor
/// graph. The result is deduplicated but unordered. Does not include `target`
/// itself.
///
/// Stops at changes that have been trimmed from the log (trimmed ranges are
/// represented as tombstones in the log; the BFS halts when it encounters one).
///
/// Complexity: O(ancestors).
pub fn causal_ancestors(world: &World, target: ChangeId) -> Vec<ChangeId> {
    world
        .log()
        .causal_ancestors(target)
        .into_iter()
        .map(|c| c.id)
        .collect()
}

/// True iff `ancestor` is a causal ancestor of `descendant` in the change
/// log. Equivalent to `causal_ancestors(world, descendant).contains(&ancestor)`
/// but short-circuits on the first confirming path found.
///
/// Returns `false` if either ID is not in the log.
pub fn is_ancestor_of(world: &World, ancestor: ChangeId, descendant: ChangeId) -> bool {
    world.log().is_ancestor_of(ancestor, descendant)
}

// ─── Locus change range ───────────────────────────────────────────────────────

/// All changes that affected `locus` within the (inclusive) batch range
/// `[from_batch, to_batch]`, newest first.
///
/// Wraps `ChangeLog::changes_to_locus` with a batch-range filter. Useful for
/// auditing what happened to a specific locus over a time window.
pub fn changes_to_locus_in_range(
    world: &World,
    locus: LocusId,
    from_batch: BatchId,
    to_batch: BatchId,
) -> Vec<&Change> {
    world
        .changes_to_locus(locus)
        .filter(|c| c.batch.0 >= from_batch.0 && c.batch.0 <= to_batch.0)
        .collect()
}

/// All changes that affected `rel` within the batch range `[from_batch,
/// to_batch]`, newest first.
pub fn changes_to_relationship_in_range(
    world: &World,
    rel: RelationshipId,
    from_batch: BatchId,
    to_batch: BatchId,
) -> Vec<&Change> {
    world
        .changes_to_relationship(rel)
        .filter(|c| c.batch.0 >= from_batch.0 && c.batch.0 <= to_batch.0)
        .collect()
}

// ─── Forward causal walk ─────────────────────────────────────────────────────

/// All changes that have `target` as a direct or transitive causal predecessor
/// — the "forward" dual of [`causal_ancestors`].
///
/// Because the `ChangeLog` only stores backward (predecessor) links, this
/// requires an O(N) scan of the full log to build a forward-edge index, then a
/// BFS from `target`. For large logs this can be expensive; use on trimmed or
/// bounded logs where appropriate.
///
/// The result is deduplicated but unordered. `target` itself is not included.
/// Returns an empty `Vec` when `target` has no descendants (e.g. it is the
/// most recent change, or it is not in the log).
pub fn causal_descendants(world: &World, target: ChangeId) -> Vec<ChangeId> {
    use rustc_hash::FxHashMap;
    use std::collections::VecDeque;

    // Build forward adjacency: predecessor → Vec<successor>.
    let mut forward: FxHashMap<ChangeId, Vec<ChangeId>> = FxHashMap::default();
    for c in world.log().iter() {
        for &pred in &c.predecessors {
            forward.entry(pred).or_default().push(c.id);
        }
    }

    // BFS forward from target.
    let mut visited: FxHashSet<ChangeId> = FxHashSet::default();
    let mut queue: VecDeque<ChangeId> = VecDeque::new();
    if let Some(children) = forward.get(&target) {
        for &c in children {
            if visited.insert(c) {
                queue.push_back(c);
            }
        }
    }
    while let Some(cid) = queue.pop_front() {
        if let Some(children) = forward.get(&cid) {
            for &c in children {
                if visited.insert(c) {
                    queue.push_back(c);
                }
            }
        }
    }
    visited.into_iter().collect()
}

// ─── Source trace ─────────────────────────────────────────────────────────────

/// Walk backwards from `target` to find all root changes (stimuli — changes
/// with no predecessors) that are ancestors of `target`.
///
/// These are the external inputs that ultimately caused `target` to fire.
/// Returns an empty `Vec` when `target` itself is a stimulus.
pub fn root_stimuli(world: &World, target: ChangeId) -> Vec<ChangeId> {
    let ancestors = causal_ancestors(world, target);
    ancestors
        .into_iter()
        .filter(|&cid| {
            world
                .log()
                .get(cid)
                .is_some_and(|c| c.predecessors.is_empty())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{
        BatchId, Change, ChangeId, ChangeSubject, InfluenceKindId, LocusId, StateVector,
    };
    use graph_world::World;

    fn push_change(
        world: &mut World,
        id: u64,
        locus: u64,
        preds: Vec<u64>,
        batch: u64,
    ) -> ChangeId {
        let cid = ChangeId(id);
        world.log_mut().append(Change {
            id: cid,
            subject: ChangeSubject::Locus(LocusId(locus)),
            kind: InfluenceKindId(1),
            predecessors: preds.into_iter().map(ChangeId).collect(),
            before: StateVector::zeros(1),
            after: StateVector::from_slice(&[0.5]),
            batch: BatchId(batch),
            wall_time: None,
            metadata: None,
        });
        cid
    }

    // Causal chain: c0 (root) → c1 → c2
    fn chain_world() -> World {
        let mut w = World::new();
        push_change(&mut w, 0, 0, vec![], 0);
        push_change(&mut w, 1, 1, vec![0], 1);
        push_change(&mut w, 2, 2, vec![1], 2);
        w
    }

    #[test]
    fn causal_ancestors_returns_all_predecessors() {
        let w = chain_world();
        let mut ancestors = causal_ancestors(&w, ChangeId(2));
        ancestors.sort();
        assert_eq!(ancestors, vec![ChangeId(0), ChangeId(1)]);
    }

    #[test]
    fn causal_ancestors_of_root_is_empty() {
        let w = chain_world();
        assert!(causal_ancestors(&w, ChangeId(0)).is_empty());
    }

    #[test]
    fn is_ancestor_of_detects_transitivity() {
        let w = chain_world();
        assert!(is_ancestor_of(&w, ChangeId(0), ChangeId(2)));
        assert!(is_ancestor_of(&w, ChangeId(1), ChangeId(2)));
        assert!(!is_ancestor_of(&w, ChangeId(2), ChangeId(0)));
    }

    #[test]
    fn changes_to_locus_in_range_filters_by_batch() {
        let mut w = World::new();
        // Three changes to locus 0, at batches 1, 3, 5.
        push_change(&mut w, 0, 0, vec![], 1);
        push_change(&mut w, 1, 0, vec![], 3);
        push_change(&mut w, 2, 0, vec![], 5);

        let range = changes_to_locus_in_range(&w, LocusId(0), BatchId(2), BatchId(4));
        assert_eq!(range.len(), 1);
        assert_eq!(range[0].batch, BatchId(3));
    }

    #[test]
    fn root_stimuli_finds_origin_changes() {
        let w = chain_world();
        let roots = root_stimuli(&w, ChangeId(2));
        assert_eq!(roots, vec![ChangeId(0)]);
    }

    #[test]
    fn root_stimuli_empty_for_stimulus_itself() {
        let w = chain_world();
        assert!(root_stimuli(&w, ChangeId(0)).is_empty());
    }

    // ── root_stimuli_for_relationship ────────────────────────────────────────

    fn world_with_relationship_created_by(
        created_by: Option<u64>,
        root_pred: Vec<u64>,
    ) -> (World, RelationshipId) {
        use graph_core::{
            Endpoints, InfluenceKindId, KindObservation, Locus, LocusKindId, Relationship,
            RelationshipKindId, RelationshipLineage, StateVector,
        };
        let mut w = World::new();
        let rk: RelationshipKindId = InfluenceKindId(1);
        w.insert_locus(graph_core::Locus::new(
            LocusId(0),
            LocusKindId(1),
            StateVector::zeros(1),
        ));
        w.insert_locus(Locus::new(
            LocusId(1),
            LocusKindId(1),
            StateVector::zeros(1),
        ));

        // Push a root change (no predecessors) at id 0.
        push_change(&mut w, 0, 0, vec![], 0);
        // Push a derived change with `root_pred` as predecessors.
        if let Some(cid) = created_by {
            push_change(&mut w, cid, 1, root_pred, 1);
        }

        let rel_id = w.relationships_mut().mint_id();
        w.relationships_mut().insert(Relationship {
            id: rel_id,
            kind: rk,
            endpoints: Endpoints::Directed {
                from: LocusId(0),
                to: LocusId(1),
            },
            state: StateVector::from_slice(&[1.0, 0.0]),
            lineage: RelationshipLineage {
                created_by: created_by.map(ChangeId),
                last_touched_by: None,
                change_count: 1,
                kinds_observed: smallvec::smallvec![KindObservation::synthetic(rk)],
            },
            created_batch: graph_core::BatchId(0),
            last_decayed_batch: 0,
            metadata: None,
        });
        (w, rel_id)
    }

    #[test]
    fn root_stimuli_for_relationship_returns_empty_when_no_created_by() {
        let (w, rel_id) = world_with_relationship_created_by(None, vec![]);
        assert!(root_stimuli_for_relationship(&w, rel_id).is_empty());
    }

    #[test]
    fn root_stimuli_for_relationship_returns_stimulus_when_created_by_is_root() {
        // Change 1 has no predecessors → it IS the root stimulus.
        let (w, rel_id) = world_with_relationship_created_by(Some(1), vec![]);
        let roots = root_stimuli_for_relationship(&w, rel_id);
        assert_eq!(roots, vec![ChangeId(1)]);
    }

    #[test]
    fn root_stimuli_for_relationship_traces_through_predecessors() {
        // Chain: c0 (root) → c1 → c2 (created_by for rel)
        let mut w = World::new();
        push_change(&mut w, 0, 0, vec![], 0); // root
        push_change(&mut w, 1, 1, vec![0], 1); // derived
        push_change(&mut w, 2, 2, vec![1], 2); // created_by

        use graph_core::{
            Endpoints, InfluenceKindId, KindObservation, Locus, LocusKindId, Relationship,
            RelationshipKindId, RelationshipLineage, StateVector,
        };
        let rk: RelationshipKindId = InfluenceKindId(1);
        for i in 0..3 {
            w.insert_locus(Locus::new(
                LocusId(i),
                LocusKindId(1),
                StateVector::zeros(1),
            ));
        }
        let rel_id = w.relationships_mut().mint_id();
        w.relationships_mut().insert(Relationship {
            id: rel_id,
            kind: rk,
            endpoints: Endpoints::Directed {
                from: LocusId(0),
                to: LocusId(1),
            },
            state: StateVector::from_slice(&[1.0, 0.0]),
            lineage: RelationshipLineage {
                created_by: Some(ChangeId(2)),
                last_touched_by: None,
                change_count: 1,
                kinds_observed: smallvec::smallvec![KindObservation::synthetic(rk)],
            },
            created_batch: graph_core::BatchId(0),
            last_decayed_batch: 0,
            metadata: None,
        });

        let roots = root_stimuli_for_relationship(&w, rel_id);
        assert_eq!(roots, vec![ChangeId(0)]);
    }

    // ── relationship_volatility ──────────────────────────────────────────────

    fn world_with_rel_changes(activity_values: &[f32]) -> (World, RelationshipId) {
        use graph_core::{
            Change, ChangeSubject, Endpoints, InfluenceKindId, KindObservation, Locus, LocusKindId,
            Relationship, RelationshipKindId, RelationshipLineage, StateVector,
        };
        let rk: RelationshipKindId = InfluenceKindId(1);
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

        let rel_id = w.relationships_mut().mint_id();
        w.relationships_mut().insert(Relationship {
            id: rel_id,
            kind: rk,
            endpoints: Endpoints::Directed {
                from: LocusId(0),
                to: LocusId(1),
            },
            state: StateVector::from_slice(&[1.0, 0.0]),
            lineage: RelationshipLineage {
                created_by: None,
                last_touched_by: None,
                change_count: activity_values.len() as u64,
                kinds_observed: smallvec::smallvec![KindObservation::synthetic(rk)],
            },
            created_batch: graph_core::BatchId(0),
            last_decayed_batch: 0,
            metadata: None,
        });

        for (batch, &act) in activity_values.iter().enumerate() {
            let cid = w.mint_change_id();
            w.append_change(Change {
                id: cid,
                subject: ChangeSubject::Relationship(rel_id),
                kind: InfluenceKindId(1),
                predecessors: vec![],
                before: StateVector::from_slice(&[0.0, 0.0]),
                after: StateVector::from_slice(&[act, 0.0]),
                batch: BatchId(batch as u64),
                wall_time: None,
                metadata: None,
            });
        }
        (w, rel_id)
    }

    #[test]
    fn relationship_volatility_zero_for_fewer_than_two_changes() {
        let (w, rel_id) = world_with_rel_changes(&[0.5]);
        assert_eq!(
            relationship_volatility(&w, rel_id, BatchId(0), BatchId(10)),
            0.0
        );
    }

    #[test]
    fn relationship_volatility_zero_for_constant_activity() {
        let (w, rel_id) = world_with_rel_changes(&[0.5, 0.5, 0.5]);
        let v = relationship_volatility(&w, rel_id, BatchId(0), BatchId(10));
        assert!(
            v.abs() < 1e-5,
            "constant activity should have ~0 volatility, got {v}"
        );
    }

    #[test]
    fn relationship_volatility_nonzero_for_variable_activity() {
        let (w, rel_id) = world_with_rel_changes(&[0.1, 0.9, 0.1, 0.9]);
        let v = relationship_volatility(&w, rel_id, BatchId(0), BatchId(10));
        assert!(
            v > 0.3,
            "alternating activity should have high volatility, got {v}"
        );
    }

    // ── loci_changed_in_batch / relationships_changed_in_batch ──────────────

    fn push_rel_change(world: &mut World, id: u64, rel: u64, batch: u64) {
        use graph_core::RelationshipId;
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
    fn loci_changed_in_batch_returns_unique_loci() {
        let mut w = World::new();
        // Batch 1: locus 0 twice, locus 1 once
        push_change(&mut w, 0, 0, vec![], 1);
        push_change(&mut w, 1, 0, vec![], 1);
        push_change(&mut w, 2, 1, vec![], 1);
        // Batch 2: locus 2
        push_change(&mut w, 3, 2, vec![], 2);

        let mut batch1 = loci_changed_in_batch(&w, BatchId(1));
        batch1.sort();
        assert_eq!(batch1, vec![LocusId(0), LocusId(1)]);

        let batch2 = loci_changed_in_batch(&w, BatchId(2));
        assert_eq!(batch2, vec![LocusId(2)]);

        let empty = loci_changed_in_batch(&w, BatchId(99));
        assert!(empty.is_empty());
    }

    #[test]
    fn relationships_changed_in_batch_excludes_locus_changes() {
        let mut w = World::new();
        push_change(&mut w, 0, 0, vec![], 1); // locus change — should be excluded
        push_rel_change(&mut w, 1, 10, 1); // rel 10 in batch 1
        push_rel_change(&mut w, 2, 10, 1); // rel 10 again — deduplicated
        push_rel_change(&mut w, 3, 20, 1); // rel 20 in batch 1

        use graph_core::RelationshipId;
        let mut rels = relationships_changed_in_batch(&w, BatchId(1));
        rels.sort();
        assert_eq!(rels, vec![RelationshipId(10), RelationshipId(20)]);
    }

    // ── common_ancestors ────────────────────────────────────────────────────

    #[test]
    fn common_ancestors_finds_shared_root() {
        // Diamond: c0 → c1 → c3
        //          c0 → c2 → c3
        let mut w = World::new();
        push_change(&mut w, 0, 0, vec![], 0); // root
        push_change(&mut w, 1, 1, vec![0], 1);
        push_change(&mut w, 2, 2, vec![0], 1);
        push_change(&mut w, 3, 3, vec![1, 2], 2);

        let mut common = common_ancestors(&w, ChangeId(1), ChangeId(2));
        common.sort();
        assert_eq!(common, vec![ChangeId(0)]);
    }

    #[test]
    fn common_ancestors_empty_for_disjoint_chains() {
        let mut w = World::new();
        push_change(&mut w, 0, 0, vec![], 0); // chain A root
        push_change(&mut w, 1, 1, vec![0], 1);
        push_change(&mut w, 2, 2, vec![], 0); // chain B root (independent)
        push_change(&mut w, 3, 3, vec![2], 1);

        let common = common_ancestors(&w, ChangeId(1), ChangeId(3));
        assert!(common.is_empty());
    }

    // ── causal_coarse_trail ─────────────────────────────────────────────────

    #[test]
    fn coarse_trail_exact_when_no_trim() {
        let w = chain_world();
        let trail = causal_coarse_trail(&w, ChangeId(2));
        assert!(trail.is_exact(), "no trim → should be exact");
        assert!(!trail.fine.is_empty(), "should have fine ancestors");
    }

    #[test]
    fn coarse_trail_contains_summary_after_trim() {
        let mut w = World::new();
        // c0 (batch 0, locus 0) → c1 (batch 1) → c2 (batch 2)
        push_change(&mut w, 0, 0, vec![], 0);
        push_change(&mut w, 1, 1, vec![0], 1);
        push_change(&mut w, 2, 2, vec![1], 2);

        // Trim everything before batch 1 — c0 becomes a summary for locus 0.
        w.log_mut().trim_before_batch(BatchId(1));

        let trail = causal_coarse_trail(&w, ChangeId(2));
        // c1 is in fine (batch 1 was kept); c0 is trimmed.
        assert!(trail.fine.contains(&ChangeId(1)));
        // Coarse should contain the summary for locus 0 (c0's locus).
        // c1's subject is locus 1, and its predecessor c0 is trimmed → locus 1 gets a summary.
        assert!(
            !trail.coarse.is_empty(),
            "should have coarse summaries for the trimmed locus"
        );
        assert!(!trail.is_exact(), "trim boundary was crossed → not exact");
    }

    #[test]
    fn coarse_trail_empty_for_unknown_change() {
        let w = chain_world();
        let trail = causal_coarse_trail(&w, ChangeId(999));
        assert!(trail.is_empty());
    }

    // ── causal_depth ────────────────────────────────────────────────────────

    #[test]
    fn causal_depth_of_root_is_zero() {
        let w = chain_world();
        assert_eq!(causal_depth(&w, ChangeId(0)), 0);
    }

    #[test]
    fn causal_depth_follows_longest_chain() {
        // c0 (root) → c1 → c2, so depth(c2) = 2
        let w = chain_world();
        assert_eq!(causal_depth(&w, ChangeId(1)), 1);
        assert_eq!(causal_depth(&w, ChangeId(2)), 2);
    }

    #[test]
    fn causal_depth_on_diamond_takes_longer_branch() {
        // c0 → c1 → c3 (depth 2) and c0 → c2 → c3 (also depth 2)
        let mut w = World::new();
        push_change(&mut w, 0, 0, vec![], 0); // depth 0
        push_change(&mut w, 1, 1, vec![0], 1); // depth 1
        push_change(&mut w, 2, 2, vec![0], 1); // depth 1
        push_change(&mut w, 3, 3, vec![1, 2], 2); // depth 2
        assert_eq!(causal_depth(&w, ChangeId(3)), 2);
    }
}
