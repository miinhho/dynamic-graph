use graph_core::{Channel, EntityId, EntityKindId};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SelectorMode {
    ExplicitOnly,
    #[default]
    IndexedOnly,
    AllowFullScan,
}

#[derive(Debug, Clone, Default)]
pub struct EntitySelector {
    pub source: EntityId,
    pub targets: Vec<EntityId>,
    pub target_kinds: Vec<EntityKindId>,
    pub radius: Option<f32>,
    pub mode: SelectorMode,
}

#[derive(Debug, Clone, Default)]
pub struct ResolvedSelection {
    pub targets: Vec<EntityId>,
    pub cohort_kinds: Vec<EntityKindId>,
}

impl EntitySelector {
    pub fn from_channel(channel: &Channel) -> Self {
        Self {
            source: channel.source,
            targets: channel.targets.clone(),
            target_kinds: channel.target_kinds.clone(),
            radius: channel.field_radius,
            mode: SelectorMode::IndexedOnly,
        }
    }

    pub fn with_mode(mut self, mode: SelectorMode) -> Self {
        self.mode = mode;
        self
    }
}

impl ResolvedSelection {
    pub fn truncate_targets(&self, limit: usize) -> (Vec<EntityId>, usize) {
        let available = self.targets.len();
        let mut targets = self.targets.clone();
        targets.truncate(limit);
        (targets, available)
    }

    pub fn truncate_cohort_kinds(&self, limit: usize) -> (Vec<EntityKindId>, usize) {
        let available = self.cohort_kinds.len();
        let mut kinds = self.cohort_kinds.clone();
        kinds.truncate(limit);
        (kinds, available)
    }
}
