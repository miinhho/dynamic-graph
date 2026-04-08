use graph_core::{CauseId, EntityId, EntityState, TickId, WorldVersion};

use crate::{DeltaProvenance, RecordedDelta};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionIntent {
    ExternalCommand,
    Simulate,
}

#[derive(Debug, Clone)]
pub struct TickTransaction {
    pub tick: TickId,
    pub intent: TransactionIntent,
    pub read_version: WorldVersion,
    pub committed_version: Option<WorldVersion>,
    pub conflict: Option<TransactionConflict>,
    pub deltas: Vec<RecordedDelta>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransactionConflict {
    pub expected: WorldVersion,
    pub actual: WorldVersion,
}

impl TickTransaction {
    pub fn new(tick: TickId, intent: TransactionIntent, read_version: WorldVersion) -> Self {
        Self {
            tick,
            intent,
            read_version,
            committed_version: None,
            conflict: None,
            deltas: Vec::new(),
        }
    }

    pub fn simulate(tick: TickId, read_version: WorldVersion) -> Self {
        Self::new(tick, TransactionIntent::Simulate, read_version)
    }

    pub fn record(
        &mut self,
        entity: EntityId,
        before: EntityState,
        after: EntityState,
        provenance: DeltaProvenance,
    ) {
        self.deltas.push(RecordedDelta {
            entity,
            before,
            after,
            cause: CauseId(self.deltas.len() as u64 + 1),
            provenance,
        });
    }

    pub fn mark_committed(&mut self, version: WorldVersion) {
        self.committed_version = Some(version);
        self.conflict = None;
    }

    pub fn mark_conflict(&mut self, expected: WorldVersion, actual: WorldVersion) {
        self.committed_version = None;
        self.conflict = Some(TransactionConflict { expected, actual });
    }
}
