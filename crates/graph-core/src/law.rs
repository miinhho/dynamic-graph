use crate::ids::CauseId;
use crate::medium::Channel;
use crate::state::{EntityState, Stimulus};
use crate::value::SignalVector;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum InteractionKind {
    Neutral,
    Excitatory,
    Inhibitory,
}

pub trait EntityProgram: Send + Sync {
    fn next_state(
        &self,
        current: &EntityState,
        inbox: &SignalVector,
        stimulus: Option<&Stimulus>,
    ) -> EntityState;

    fn outbound_signal(&self, current: &EntityState) -> SignalVector {
        current.emitted.clone()
    }
}

pub trait EmissionLaw: Send + Sync {
    fn project(
        &self,
        outbound: &SignalVector,
        source: &EntityState,
        channel: &Channel,
    ) -> SignalVector;

    fn cause(&self, channel: &Channel) -> CauseId {
        CauseId(channel.id.0)
    }

    fn kind(&self) -> InteractionKind {
        InteractionKind::Neutral
    }
}
