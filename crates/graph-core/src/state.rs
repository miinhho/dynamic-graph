use crate::ids::{EntityId, EntityKindId};
use crate::value::{SignalVector, StateVector};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct EntityState {
    pub internal: StateVector,
    pub emitted: SignalVector,
    pub cooldown: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entity {
    pub id: EntityId,
    pub kind: EntityKindId,
    pub position: StateVector,
    pub state: EntityState,
    pub refractory_period: u32,
    pub budget: crate::budget::EmissionBudget,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Stimulus {
    pub target: EntityId,
    pub signal: SignalVector,
}
