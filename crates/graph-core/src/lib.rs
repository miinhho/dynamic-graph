//! graph-core: foundational primitives for the substrate.
//!
//! See `docs/redesign.md` for design rationale and `docs/identity.md`
//! for the settled ontology. Exposes all five layers:
//! Layer 0 (Locus), Layer 1 (Change), Layer 2 (Relationship),
//! Layer 3 (Entity), Layer 4 (Cohere), plus shared support types
//! (StateVector, stabilization, weathering).

pub mod change;
pub mod cohere;
pub mod entity;
pub mod ids;
pub mod locus;
pub mod perspective;
pub mod program;
pub mod relationship;
pub mod stabilization;
pub mod state;
pub mod weathering;

pub use change::{Change, ChangeSubject};
pub use ids::{BatchId, ChangeId, InfluenceKindId, LocusId, LocusKindId};
pub use locus::Locus;
pub use program::{LocusProgram, ProposedChange, StructuralProposal};
pub use cohere::{Cohere, CohereId, CohereMembers};
pub use entity::{
    CompressedTransition, CompressionLevel, Entity, EntityId, EntityLayer, EntityLineage,
    EntitySnapshot, EntityStatus, LayerTransition,
};
pub use perspective::EmergenceProposal;
pub use stabilization::{SaturationMode, StabilizationConfig};
pub use relationship::{
    EndpointKey, Endpoints, Relationship, RelationshipId, RelationshipKindId, RelationshipLineage,
};
pub use state::StateVector;
pub use weathering::{
    apply_compress, apply_skeleton, DefaultEntityWeathering, EntityWeatheringPolicy,
    WeatheringEffect,
};
