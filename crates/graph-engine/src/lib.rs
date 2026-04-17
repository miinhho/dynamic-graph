//! graph-engine: substrate batch loop and emergent layers.
//!
//! See `docs/redesign.md` for design rationale and `docs/identity.md`
//! for the settled ontology. Owns the batch loop (`Engine::tick`), kind
//! registries, regime classification, adaptive guard rail, and the
//! emergence / cohere perspectives.

mod cohere;
mod controller;
mod emergence;
mod engine;
mod handle;
mod regime;
mod registry;
mod simulation;

pub use cohere::{CoherePerspective, DefaultCoherePerspective};
pub use emergence::{DefaultEmergencePerspective, EmergencePerspective};
pub use regime::{
    AdaptiveConfig, AdaptiveGuardRail,
    BatchHistory, BatchMetrics, DefaultRegimeClassifier, DynamicsRegime, RegimeClassifier,
};
pub use engine::{Engine, EngineConfig, TickResult};
pub use graph_core::{DefaultEntityWeathering, Encoder, EntityWeatheringPolicy, LifecycleCause, PassthroughEncoder, Properties, PropertyValue, RegimeTag, StructuralProposal, WeatheringEffect, WorldEvent};
pub use registry::{InfluenceKindConfig, InfluenceKindRegistry, LocusKindConfig, LocusKindRegistry, PlasticityConfig, SlotDefsMap};
pub use graph_core::RelationshipSlotDef;
pub use graph_world::{SubscriptionStore, WorldDiff, WorldMetrics};
pub use simulation::{
    BackpressurePolicy, EventHistory, IngestError,
    Simulation, SimulationBuilder, SimulationConfig, StepObservation, TickSummary,
};
pub use handle::{EngineHandle, LocalHandle};
pub use controller::{EngineController, TickPolicy};
