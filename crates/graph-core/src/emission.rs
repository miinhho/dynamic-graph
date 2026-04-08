use crate::ids::{CauseId, ChannelId, EntityId, LawId};
use crate::law::InteractionKind;
use crate::value::SignalVector;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmissionOrigin {
    pub source: EntityId,
    pub target: EntityId,
    pub channel: ChannelId,
    pub law: LawId,
    pub kind: InteractionKind,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Emission {
    pub signal: SignalVector,
    pub magnitude: f32,
    pub cause: CauseId,
    pub origin: Option<EmissionOrigin>,
}
