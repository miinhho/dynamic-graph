use graph_core::{ChannelId, EntityId, InteractionKind, LawId, TickId};

#[derive(Debug, Clone, Default)]
pub struct TickDiagnostics {
    pub tick: TickId,
    pub active_entities: Vec<EntityId>,
    pub emitted_channels: Vec<ChannelId>,
    pub total_emissions: usize,
    pub fanout_capped_entities: Vec<EntityId>,
    pub interaction_kinds: Vec<InteractionKind>,
    pub law_ids: Vec<LawId>,
    pub promoted_to_field: Vec<ChannelId>,
    pub promoted_to_cohort: Vec<ChannelId>,
}
