use graph_core::{CauseId, EntityId, TickId, WorldVersion};

use crate::{RecordedDelta, TickTransaction};

#[derive(Clone, Copy)]
pub struct TransactionQuery<'a> {
    transaction: &'a TickTransaction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransactionSummary {
    pub tick: TickId,
    pub read_version: WorldVersion,
    pub committed_version: Option<WorldVersion>,
    pub delta_count: usize,
    pub has_conflict: bool,
}

impl<'a> TransactionQuery<'a> {
    pub fn new(transaction: &'a TickTransaction) -> Self {
        Self { transaction }
    }

    pub fn transaction(&self) -> &'a TickTransaction {
        self.transaction
    }

    pub fn summary(&self) -> TransactionSummary {
        TransactionSummary {
            tick: self.transaction.tick,
            read_version: self.transaction.read_version,
            committed_version: self.transaction.committed_version,
            delta_count: self.transaction.deltas.len(),
            has_conflict: self.transaction.conflict.is_some(),
        }
    }

    pub fn deltas(&self) -> &'a [RecordedDelta] {
        &self.transaction.deltas
    }

    pub fn deltas_for_entity(&self, entity: EntityId) -> Vec<&'a RecordedDelta> {
        self.transaction
            .deltas
            .iter()
            .filter(|delta| delta.entity == entity)
            .collect()
    }

    pub fn delta_by_cause(&self, cause: CauseId) -> Option<&'a RecordedDelta> {
        self.transaction
            .deltas
            .iter()
            .find(|delta| delta.cause == cause)
    }

    pub fn changed_entities(&self) -> Vec<EntityId> {
        let mut entities = self
            .transaction
            .deltas
            .iter()
            .map(|delta| delta.entity)
            .collect::<Vec<_>>();
        entities.sort_unstable_by_key(|id| id.0);
        entities.dedup_by_key(|id| id.0);
        entities
    }
}
