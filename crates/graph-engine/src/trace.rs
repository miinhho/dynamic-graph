use graph_core::{ChannelId, EntityId, TickId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraceEvent {
    TickStarted {
        tick: TickId,
        active_entities: Vec<EntityId>,
    },
    SourceDispatched {
        tick: TickId,
        source: EntityId,
        emitted_channels: Vec<ChannelId>,
        total_emissions: usize,
    },
    StateComputed {
        tick: TickId,
        entity: EntityId,
    },
    DeltaRecorded {
        tick: TickId,
        entity: EntityId,
        source_entities: Vec<EntityId>,
        channel_ids: Vec<ChannelId>,
    },
    TickCommitted {
        tick: TickId,
        committed_entities: Vec<EntityId>,
    },
    TickConflicted {
        tick: TickId,
    },
}

pub trait TraceSink {
    fn should_record(&self, _: &TraceEvent) -> bool {
        true
    }

    fn record(&mut self, event: TraceEvent);
}

#[derive(Default)]
pub struct NoopTraceSink;

impl TraceSink for NoopTraceSink {
    fn record(&mut self, _: TraceEvent) {}
}
