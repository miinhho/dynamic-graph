use graph_core::{CauseId, Channel, ChannelId, Entity, EntityId, EntityKindId};
use graph_tx::{RecordedDelta, TickTransaction, TransactionQuery, TransactionSummary};
use graph_world::{
    EntityProjection, EntitySelector, ResolvedSelection, SnapshotQuery, WorldSnapshot,
};

use crate::{RuntimeTick, TickDiagnostics, TickResult};

#[derive(Clone, Copy)]
pub struct TickInspection<'a> {
    snapshot_query: SnapshotQuery<'a>,
    transaction_query: TransactionQuery<'a>,
    diagnostics: &'a TickDiagnostics,
}

impl<'a> TickInspection<'a> {
    pub fn new(
        snapshot: WorldSnapshot<'a>,
        transaction: &'a TickTransaction,
        diagnostics: &'a TickDiagnostics,
    ) -> Self {
        Self {
            snapshot_query: SnapshotQuery::new(snapshot),
            transaction_query: TransactionQuery::new(transaction),
            diagnostics,
        }
    }

    pub fn snapshot_query(&self) -> SnapshotQuery<'a> {
        self.snapshot_query
    }

    pub fn transaction_query(&self) -> TransactionQuery<'a> {
        self.transaction_query
    }

    pub fn diagnostics(&self) -> &'a TickDiagnostics {
        self.diagnostics
    }

    pub fn entity(&self, id: EntityId) -> Option<&'a Entity> {
        self.snapshot_query.entity(id)
    }

    pub fn entity_projection(&self, id: EntityId) -> Option<EntityProjection<'a>> {
        self.snapshot_query.entity_projection(id)
    }

    pub fn entities_by_kind(&self, kind: EntityKindId) -> Vec<&'a Entity> {
        self.snapshot_query.entities_by_kind(kind)
    }

    pub fn select(&self, selector: &EntitySelector) -> ResolvedSelection {
        self.snapshot_query.select(selector)
    }

    pub fn selected_entities(&self, selector: &EntitySelector) -> Vec<&'a Entity> {
        self.snapshot_query.selected_entities(selector)
    }

    pub fn selected_projections(&self, selector: &EntitySelector) -> Vec<EntityProjection<'a>> {
        self.snapshot_query.selected_projections(selector)
    }

    pub fn channels_from(&self, source: EntityId) -> impl Iterator<Item = &'a Channel> {
        self.snapshot_query.channels_from(source)
    }

    pub fn channels_to(&self, target: EntityId) -> Vec<&'a Channel> {
        self.snapshot_query.channels_to(target)
    }

    pub fn count_entities(&self) -> usize {
        self.snapshot_query.count_entities()
    }

    pub fn count_channels(&self) -> usize {
        self.snapshot_query.count_channels()
    }

    pub fn transaction_summary(&self) -> TransactionSummary {
        self.transaction_query.summary()
    }

    pub fn deltas(&self) -> &'a [RecordedDelta] {
        self.transaction_query.deltas()
    }

    pub fn deltas_for_entity(&self, entity: EntityId) -> Vec<&'a RecordedDelta> {
        self.transaction_query.deltas_for_entity(entity)
    }

    pub fn delta_by_cause(&self, cause: CauseId) -> Option<&'a RecordedDelta> {
        self.transaction_query.delta_by_cause(cause)
    }

    pub fn changed_entities(&self) -> Vec<EntityId> {
        self.transaction_query.changed_entities()
    }

    pub fn changed_entity_projections(&self) -> Vec<EntityProjection<'a>> {
        self.changed_entities()
            .into_iter()
            .filter_map(|entity_id| self.entity_projection(entity_id))
            .collect()
    }

    pub fn changed_channels(&self) -> Vec<ChannelId> {
        let mut channel_ids = self
            .deltas()
            .iter()
            .flat_map(|delta| delta.provenance.channel_ids.iter().copied())
            .collect::<Vec<_>>();
        channel_ids.sort_unstable_by_key(|id| id.0);
        channel_ids.dedup_by_key(|id| id.0);
        channel_ids
    }
}

impl TickResult {
    pub fn inspect<'a>(&'a self, snapshot: WorldSnapshot<'a>) -> TickInspection<'a> {
        TickInspection::new(snapshot, &self.transaction, &self.diagnostics)
    }
}

impl RuntimeTick {
    pub fn inspect<'a>(&'a self, snapshot: WorldSnapshot<'a>) -> TickInspection<'a> {
        self.result.inspect(snapshot)
    }
}
