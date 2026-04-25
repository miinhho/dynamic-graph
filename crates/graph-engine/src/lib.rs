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
mod plasticity;
mod regime;
mod registry;
mod simulation;

pub use self::{
    cohere::{CoherePerspective, DefaultCoherePerspective},
    controller::{EngineController, TickPolicy},
    emergence::{
        DefaultEmergencePerspective, EmergencePerspective, debug_exclusivity_counters,
        debug_last_component_count, reset_exclusivity_counters,
    },
    engine::{
        Engine, EngineConfig, TickResult, last_recognize_passes,
        last_recognize_unconverged_proposals,
    },
    handle::{EngineHandle, LocalHandle},
    plasticity::{
        PairObservationTargets, PairObservationWindow, PairPredictionObjective,
        PairPredictionRanking, PlasticityLearners, PlasticityObservation, RankedPair,
    },
    regime::{
        AdaptiveConfig, AdaptiveGuardRail, BatchHistory, BatchMetrics, DefaultRegimeClassifier,
        DynamicsRegime, Learnable, PerKindLearnable, RegimeClassifier,
    },
    registry::{
        DemotionPolicy, EmergenceThreshold, InfluenceKindConfig, InfluenceKindRegistry,
        LocusKindConfig, LocusKindRegistry, PlasticityConfig, SlotDefsMap,
    },
    simulation::{
        BackpressurePolicy, EventHistory, IngestError, Simulation, SimulationBuilder,
        SimulationConfig, StepObservation, TickSummary,
    },
};

pub use graph_core::{
    DefaultEntityWeathering, Encoder, EntityWeatheringPolicy, LifecycleCause, PassthroughEncoder,
    Properties, PropertyValue, RegimeTag, RelationshipSlotDef, StructuralProposal, TrimSummary,
    WeatheringEffect, WorldEvent,
};
pub use graph_world::{SubscriptionStore, WorldDiff, WorldMetrics};
