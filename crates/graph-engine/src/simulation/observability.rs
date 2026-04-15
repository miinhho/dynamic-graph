//! Simulation observability: per-tick summaries and rolling history.
//!
//! [`TickSummary`] is a richer breakdown of one `step()` call — beyond the
//! counts in [`TickResult`], it includes per-batch statistics, the distinct
//! loci that changed, and event categories. A `TickSummary` is included in
//! every [`StepObservation`] so callers have rich data without extra queries.
//!
//! [`EventHistory`] is an optional ring buffer attached to `Simulation` that
//! accumulates `TickSummary`s across steps. Enable it via
//! [`SimulationConfig::event_history_len`][crate::simulation::SimulationConfig].
//!
//! ## Example
//!
//! ```rust,ignore
//! let obs = sim.step(stimuli);
//! println!("loci active: {}", obs.summary.loci_changed.len());
//!
//! // Look back at the last 5 steps
//! for summary in sim.history().iter().rev().take(5) {
//!     println!("tick {}: {} changes", summary.tick_id, summary.changes_committed);
//! }
//! ```

use std::collections::VecDeque;

use graph_core::{BatchId, LocusId, WorldEvent};
use graph_world::World;
use graph_query::BatchStats;

use super::super::engine::TickResult;

// ─── TickSummary ─────────────────────────────────────────────────────────────

/// A detailed breakdown of one `step()` call.
///
/// Included in every [`StepObservation`][crate::simulation::config::StepObservation].
/// Produced by [`TickSummary::compute`].
#[derive(Debug, Clone)]
pub struct TickSummary {
    /// Monotone step counter (1 on the first `step()` call).
    pub tick_id: u64,
    /// Number of batches committed during this tick.
    pub batches_committed: u32,
    /// Total changes committed across all batches in this tick.
    pub changes_committed: u32,
    /// `true` if the batch cap fired before the system reached quiescence.
    pub hit_batch_cap: bool,
    /// Per-batch statistics for every batch committed this tick.
    ///
    /// Length equals `batches_committed`. Empty when no changes were committed.
    pub batch_stats: Vec<BatchStats>,
    /// Distinct loci that had at least one change committed during this tick,
    /// in undefined order.
    pub loci_changed: Vec<LocusId>,
    /// Number of relationships that auto-emerged during this tick.
    pub relationships_emerged: u32,
    /// Number of relationships that were pruned (decay-evicted) during this tick.
    pub relationships_pruned: u32,
    /// Number of entity birth events during this tick.
    pub entities_born: u32,
    /// Number of entity dormancy transitions during this tick.
    pub entities_dormant: u32,
    /// Number of entity revival transitions during this tick.
    pub entities_revived: u32,
    /// All events emitted during this tick (from the engine and the step wrapper).
    pub events: Vec<WorldEvent>,
}

impl TickSummary {
    /// Compute a `TickSummary` from the result of one tick.
    ///
    /// - `tick_id` — the monotone step counter from `Simulation`.
    /// - `tick` — the `TickResult` returned by `Engine::tick`.
    /// - `prev_batch` — the world's `current_batch` *before* the tick.
    /// - `current_batch` — the world's `current_batch` *after* the tick.
    /// - `world` — the world (to extract per-batch stats and changed loci).
    /// - `extra_events` — any additional events appended after the tick
    ///   (e.g. `RegimeShift`).
    pub fn compute(
        tick_id: u64,
        tick: &TickResult,
        prev_batch: BatchId,
        current_batch: BatchId,
        world: &World,
        extra_events: &[WorldEvent],
    ) -> Self {
        // Collect per-batch stats for all batches committed this tick.
        let batch_stats: Vec<BatchStats> = ((prev_batch.0 + 1)..=current_batch.0)
            .filter_map(|b| graph_query::batch_stats(world, BatchId(b)))
            .collect();

        // Collect distinct loci changed this tick.
        use rustc_hash::FxHashSet;
        let mut loci_set: FxHashSet<LocusId> = FxHashSet::default();
        for stat in &batch_stats {
            // Re-scan the log for loci changed in each batch.
            for change in world.log().batch(stat.batch) {
                if let graph_core::ChangeSubject::Locus(id) = change.subject {
                    loci_set.insert(id);
                }
            }
        }
        let loci_changed: Vec<LocusId> = loci_set.into_iter().collect();

        // Categorize engine events.
        let all_events: Vec<WorldEvent> = tick
            .events
            .iter()
            .cloned()
            .chain(extra_events.iter().cloned())
            .collect();

        let mut relationships_emerged = 0u32;
        let mut relationships_pruned = 0u32;
        let mut entities_born = 0u32;
        let mut entities_dormant = 0u32;
        let mut entities_revived = 0u32;

        for event in &all_events {
            match event {
                WorldEvent::RelationshipEmerged { .. } => relationships_emerged += 1,
                WorldEvent::RelationshipPruned { .. } => relationships_pruned += 1,
                WorldEvent::EntityBorn { .. } => entities_born += 1,
                WorldEvent::EntityDormant { .. } => entities_dormant += 1,
                WorldEvent::EntityRevived { .. } => entities_revived += 1,
                _ => {}
            }
        }

        TickSummary {
            tick_id,
            batches_committed: tick.batches_committed,
            changes_committed: tick.changes_committed,
            hit_batch_cap: tick.hit_batch_cap,
            batch_stats,
            loci_changed,
            relationships_emerged,
            relationships_pruned,
            entities_born,
            entities_dormant,
            entities_revived,
            events: all_events,
        }
    }
}

// ─── EventHistory ─────────────────────────────────────────────────────────────

/// A rolling ring buffer of [`TickSummary`] records.
///
/// Attached to `Simulation` when
/// [`SimulationConfig::event_history_len`][crate::simulation::SimulationConfig]
/// is greater than zero. Accessible via [`Simulation::history`].
///
/// Iteration order is oldest-first; use `.iter().rev()` for newest-first.
#[derive(Debug, Clone)]
pub struct EventHistory {
    buffer: VecDeque<TickSummary>,
    max_len: usize,
}

impl EventHistory {
    /// Create a ring buffer that retains at most `max_len` summaries.
    pub fn new(max_len: usize) -> Self {
        Self {
            buffer: VecDeque::with_capacity(max_len.min(64)),
            max_len,
        }
    }

    /// Append a summary, evicting the oldest entry if the buffer is full.
    pub fn push(&mut self, summary: TickSummary) {
        if self.max_len == 0 {
            return;
        }
        if self.buffer.len() >= self.max_len {
            self.buffer.pop_front();
        }
        self.buffer.push_back(summary);
    }

    /// Most recently appended summary, or `None` if the history is empty.
    pub fn latest(&self) -> Option<&TickSummary> {
        self.buffer.back()
    }

    /// Iterate all retained summaries, oldest first.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &TickSummary> {
        self.buffer.iter()
    }

    /// The last `n` summaries, oldest first. Returns fewer than `n` when the
    /// buffer holds fewer entries.
    pub fn window(&self, n: usize) -> impl DoubleEndedIterator<Item = &TickSummary> {
        let skip = self.buffer.len().saturating_sub(n);
        self.buffer.iter().skip(skip)
    }

    /// Number of summaries currently in the buffer.
    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    /// `true` if no summaries have been recorded yet.
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Maximum number of summaries this buffer retains.
    pub fn capacity(&self) -> usize {
        self.max_len
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{BatchId, InfluenceKindId, LocusId, WorldEvent};
    use graph_world::World;

    fn empty_tick(batches: u32, changes: u32) -> TickResult {
        TickResult {
            batches_committed: batches,
            changes_committed: changes,
            hit_batch_cap: false,
            events: Vec::new(),
        }
    }

    fn make_summary(tick_id: u64) -> TickSummary {
        let w = World::new();
        TickSummary::compute(tick_id, &empty_tick(0, 0), BatchId(0), BatchId(0), &w, &[])
    }

    // ── EventHistory ──────────────────────────────────────────────────────────

    #[test]
    fn history_push_and_latest() {
        let mut h = EventHistory::new(3);
        assert!(h.latest().is_none());
        h.push(make_summary(1));
        h.push(make_summary(2));
        assert_eq!(h.latest().unwrap().tick_id, 2);
        assert_eq!(h.len(), 2);
    }

    #[test]
    fn history_evicts_oldest_when_full() {
        let mut h = EventHistory::new(2);
        h.push(make_summary(1));
        h.push(make_summary(2));
        h.push(make_summary(3)); // should evict tick_id=1
        assert_eq!(h.len(), 2);
        let ids: Vec<_> = h.iter().map(|s| s.tick_id).collect();
        assert_eq!(ids, vec![2, 3]);
    }

    #[test]
    fn history_window_returns_last_n() {
        let mut h = EventHistory::new(10);
        for i in 1..=6 {
            h.push(make_summary(i));
        }
        let ids: Vec<_> = h.window(3).map(|s| s.tick_id).collect();
        assert_eq!(ids, vec![4, 5, 6]);
    }

    #[test]
    fn history_window_fewer_than_n_returns_all() {
        let mut h = EventHistory::new(10);
        h.push(make_summary(1));
        h.push(make_summary(2));
        assert_eq!(h.window(10).count(), 2);
    }

    #[test]
    fn history_disabled_when_max_len_zero() {
        let mut h = EventHistory::new(0);
        h.push(make_summary(1));
        assert!(h.is_empty());
    }

    // ── TickSummary::compute ──────────────────────────────────────────────────

    #[test]
    fn tick_summary_categorizes_events() {
        let w = World::new();
        let tick = TickResult {
            batches_committed: 0,
            changes_committed: 0,
            hit_batch_cap: false,
            events: vec![
                WorldEvent::RelationshipEmerged {
                    relationship: graph_core::RelationshipId(1),
                    from: LocusId(0),
                    to: LocusId(1),
                    kind: InfluenceKindId(1),
                    trigger_change_id: graph_core::ChangeId(0),
                },
                WorldEvent::RelationshipPruned {
                    relationship: graph_core::RelationshipId(2),
                },
                WorldEvent::EntityBorn {
                    entity: graph_core::EntityId(0),
                    batch: BatchId(1),
                    member_count: 2,
                },
            ],
        };
        let summary = TickSummary::compute(1, &tick, BatchId(0), BatchId(0), &w, &[]);
        assert_eq!(summary.relationships_emerged, 1);
        assert_eq!(summary.relationships_pruned, 1);
        assert_eq!(summary.entities_born, 1);
        assert_eq!(summary.entities_dormant, 0);
        assert_eq!(summary.events.len(), 3);
    }

    #[test]
    fn tick_summary_extra_events_merged() {
        let w = World::new();
        let tick = empty_tick(0, 0);
        let extra = [WorldEvent::RelationshipPruned {
            relationship: graph_core::RelationshipId(5),
        }];
        let summary = TickSummary::compute(1, &tick, BatchId(0), BatchId(0), &w, &extra);
        assert_eq!(summary.relationships_pruned, 1);
        assert_eq!(summary.events.len(), 1);
    }

    #[test]
    fn tick_summary_hit_batch_cap_propagated() {
        let w = World::new();
        let tick = TickResult {
            batches_committed: 64,
            changes_committed: 1000,
            hit_batch_cap: true,
            events: Vec::new(),
        };
        let summary = TickSummary::compute(1, &tick, BatchId(0), BatchId(64), &w, &[]);
        assert!(summary.hit_batch_cap);
        assert_eq!(summary.batches_committed, 64);
    }
}
