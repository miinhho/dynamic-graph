use graph_core::{CauseId, EntityId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CausalLink {
    pub cause: CauseId,
    pub source: EntityId,
    pub target: EntityId,
}
