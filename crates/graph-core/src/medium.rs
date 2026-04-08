use crate::ids::{ChannelId, EntityId, EntityKindId, LawId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChannelMode {
    Pairwise,
    Broadcast,
    Field,
    Cohort,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FieldKernel {
    Flat,
    Linear,
    Gaussian { sigma: f32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CohortReducer {
    Sum,
    Mean,
    Max,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Channel {
    pub id: ChannelId,
    pub source: EntityId,
    pub targets: Vec<EntityId>,
    pub target_kinds: Vec<EntityKindId>,
    pub field_radius: Option<f32>,
    pub field_kernel: FieldKernel,
    pub cohort_reducer: CohortReducer,
    pub law: LawId,
    pub kind: ChannelMode,
    pub weight: f32,
    pub attenuation: f32,
    pub enabled: bool,
}
