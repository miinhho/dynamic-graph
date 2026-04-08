//! graph-engine: substrate batch loop and emergent layers.
//!
//! See `docs/redesign.md` for design rationale and `docs/identity.md`
//! for the settled ontology. Owns the batch loop (`Engine::tick`), kind
//! registries, regime classification, adaptive guard rail, and the
//! emergence / cohere perspectives.

mod adaptive;
mod cohere;
mod emergence;
mod engine;
mod regime;
mod registry;
mod simulation;

pub use cohere::{CoherePerspective, DefaultCoherePerspective};
pub use emergence::{DefaultEmergencePerspective, EmergencePerspective};
pub use adaptive::{AdaptiveConfig, AdaptiveGuardRail};
pub use engine::{Engine, EngineConfig, TickResult};
pub use graph_core::{DefaultEntityWeathering, EntityWeatheringPolicy, StructuralProposal, WeatheringEffect};
pub use regime::{
    BatchHistory, BatchMetrics, DefaultRegimeClassifier, DynamicsRegime, RegimeClassifier,
};
pub use registry::{InfluenceKindConfig, InfluenceKindRegistry, LocusKindRegistry, PlasticityConfig};
pub use simulation::{Simulation, SimulationConfig, StepObservation};
