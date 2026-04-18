use graph_core::{BatchId, ChangeSubject, LocusId, WorldEvent};
use graph_query::BatchStats;
use graph_world::World;
use rustc_hash::FxHashSet;

pub(super) struct TickCollectedData {
    pub(super) batch_stats: Vec<BatchStats>,
    pub(super) loci_changed: Vec<LocusId>,
    pub(super) events: Vec<WorldEvent>,
}

impl TickCollectedData {
    pub(super) fn collect(
        prev_batch: BatchId,
        current_batch: BatchId,
        world: &World,
        tick_events: &[WorldEvent],
        extra_events: &[WorldEvent],
    ) -> Self {
        let batch_stats = collect_batch_stats(prev_batch, current_batch, world);
        let loci_changed = collect_changed_loci(world, &batch_stats);
        let events = collect_events(tick_events, extra_events);

        Self {
            batch_stats,
            loci_changed,
            events,
        }
    }
}

fn collect_batch_stats(
    prev_batch: BatchId,
    current_batch: BatchId,
    world: &World,
) -> Vec<BatchStats> {
    ((prev_batch.0 + 1)..=current_batch.0)
        .filter_map(|batch| graph_query::batch_stats(world, BatchId(batch)))
        .collect()
}

fn collect_changed_loci(world: &World, batch_stats: &[BatchStats]) -> Vec<LocusId> {
    let mut loci = FxHashSet::default();
    for stat in batch_stats {
        for change in world.log().batch(stat.batch) {
            if let ChangeSubject::Locus(id) = change.subject {
                loci.insert(id);
            }
        }
    }
    loci.into_iter().collect()
}

fn collect_events(tick_events: &[WorldEvent], extra_events: &[WorldEvent]) -> Vec<WorldEvent> {
    tick_events
        .iter()
        .cloned()
        .chain(extra_events.iter().cloned())
        .collect()
}
