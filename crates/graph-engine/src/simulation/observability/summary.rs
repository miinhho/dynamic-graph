use graph_core::WorldEvent;

use crate::engine::TickResult;

use super::{TickCollectedData, TickEventCounts, TickSummary};

pub(super) struct TickSummaryParts {
    tick_id: u64,
    batches_committed: u32,
    changes_committed: u32,
    hit_batch_cap: bool,
    batch_stats: Vec<graph_query::BatchStats>,
    loci_changed: Vec<graph_core::LocusId>,
    relationships_emerged: u32,
    relationships_pruned: u32,
    entities_born: u32,
    entities_dormant: u32,
    entities_revived: u32,
    events: Vec<WorldEvent>,
}

impl TickSummaryParts {
    pub(super) fn new(
        tick_id: u64,
        tick: &TickResult,
        collected: TickCollectedData,
        event_counts: TickEventCounts,
    ) -> Self {
        Self {
            tick_id,
            batches_committed: tick.batches_committed,
            changes_committed: tick.changes_committed,
            hit_batch_cap: tick.hit_batch_cap,
            batch_stats: collected.batch_stats,
            loci_changed: collected.loci_changed,
            relationships_emerged: event_counts.relationships_emerged,
            relationships_pruned: event_counts.relationships_pruned,
            entities_born: event_counts.entities_born,
            entities_dormant: event_counts.entities_dormant,
            entities_revived: event_counts.entities_revived,
            events: collected.events,
        }
    }

    pub(super) fn into_summary(self) -> TickSummary {
        TickSummary {
            tick_id: self.tick_id,
            batches_committed: self.batches_committed,
            changes_committed: self.changes_committed,
            hit_batch_cap: self.hit_batch_cap,
            batch_stats: self.batch_stats,
            loci_changed: self.loci_changed,
            relationships_emerged: self.relationships_emerged,
            relationships_pruned: self.relationships_pruned,
            entities_born: self.entities_born,
            entities_dormant: self.entities_dormant,
            entities_revived: self.entities_revived,
            events: self.events,
        }
    }
}
