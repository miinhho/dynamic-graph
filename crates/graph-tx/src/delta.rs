use graph_core::{CauseId, ChannelId, EntityId, EntityState, InteractionKind, LawId};

#[derive(Debug, Clone, Default)]
pub struct DeltaProvenance {
    pub component: Vec<EntityId>,
    pub iteration_count: u32,
    pub applied_alpha: f32,
    pub causes: Vec<CauseId>,
    pub source_entities: Vec<EntityId>,
    pub channel_ids: Vec<ChannelId>,
    pub law_ids: Vec<LawId>,
    pub interaction_kinds: Vec<InteractionKind>,
}

#[derive(Debug, Clone)]
pub struct RecordedDelta {
    pub entity: EntityId,
    pub before: EntityState,
    pub after: EntityState,
    pub cause: CauseId,
    pub provenance: DeltaProvenance,
}
