//! graph-core: foundational primitives for the substrate.
//!
//! See `docs/redesign.md` for design rationale and `docs/identity.md`
//! for the settled ontology. Exposes all five layers:
//! Layer 0 (Locus), Layer 1 (Change), Layer 2 (Relationship),
//! Layer 3 (Entity), Layer 4 (Cohere), plus shared support types
//! (StateVector, stabilization, weathering).

pub mod change;
pub mod cohere;
pub mod inbox;
pub mod encoder;
pub mod entity;
pub mod event;
pub mod ids;
pub mod locus;
pub mod perspective;
pub mod program;
pub mod program_builder;
pub mod property;
pub mod regime_tag;
pub mod relationship;
pub mod stabilization;
pub mod state;
pub mod weathering;

pub use change::{Change, ChangeSubject, TrimSummary};
pub use ids::{BatchId, ChangeId, InfluenceKindId, LocusId, LocusKindId, RelationshipKindId};
pub use locus::Locus;
pub use program::{
    changes_of_kind, locus_changes, relationship_changes, relationship_changes_of_kind,
    LocusContext, LocusProgram, ProposedChange, StructuralProposal,
};
pub use program_builder::{ComposedProgram, ProgramBuilder};
pub use cohere::{Cohere, CohereId, CohereMembers};
pub use entity::{
    CompressedTransition, CompressionLevel, Entity, EntityId, EntityLayer, EntityLineage,
    EntitySnapshot, EntityStatus, LayerTransition, LifecycleCause,
};
pub use perspective::EmergenceProposal;
pub use stabilization::{SaturationMode, StabilizationConfig};
pub use relationship::{
    EndpointKey, Endpoints, InteractionEffect, KindObservation, Relationship, RelationshipId,
    RelationshipLineage, RelationshipSlotDef,
};
pub use state::{StateSlotDef, StateVector};
pub use event::WorldEvent;
pub use property::{Properties, PropertyValue};
pub use encoder::{Encoder, PassthroughEncoder};
pub use regime_tag::RegimeTag;
pub use weathering::{
    apply_compress, apply_skeleton, DefaultEntityWeathering, EntityWeatheringPolicy,
    WeatheringEffect,
};
