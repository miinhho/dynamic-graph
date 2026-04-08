//! graph-core: foundational primitives for the substrate.
//!
//! See `docs/redesign.md` for the framing. This crate currently exposes
//! Layer 0 (Locus) and Layer 1 (Change) plus their ID newtypes and a
//! plain numeric `StateVector`. Higher-layer types (Relationship, Entity,
//! Cohere) land in follow-up commits as the substrate is rebuilt.

pub mod change;
pub mod cohere;
pub mod entity;
pub mod ids;
pub mod locus;
pub mod perspective;
pub mod program;
pub mod relationship;
pub mod state;

pub use change::{Change, ChangeSubject};
pub use ids::{BatchId, ChangeId, InfluenceKindId, LocusId, LocusKindId};
pub use locus::Locus;
pub use program::{LocusProgram, ProposedChange};
pub use cohere::{Cohere, CohereId, CohereMembers};
pub use entity::{
    CompressedTransition, CompressionLevel, Entity, EntityId, EntityLayer, EntityLineage,
    EntitySnapshot, EntityStatus, LayerTransition,
};
pub use perspective::EmergenceProposal;
pub use relationship::{
    EndpointKey, Endpoints, Relationship, RelationshipId, RelationshipKindId, RelationshipLineage,
};
pub use state::StateVector;
