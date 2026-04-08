//! graph-engine: substrate batch loop and emergent layers.
//!
//! See `docs/redesign.md` for the framing. Built layer by layer in
//! follow-up commits; this commit only exposes the kind registries the
//! batch loop will consume.

mod adaptive;
mod cohere;
mod emergence;
mod engine;
mod regime;
mod registry;

pub use cohere::{CoherePerspective, DefaultCoherePerspective};
pub use emergence::{DefaultEmergencePerspective, EmergencePerspective};
pub use adaptive::{AdaptiveConfig, AdaptiveGuardRail};
pub use engine::{Engine, EngineConfig, TickResult};
pub use graph_core::{DefaultEntityWeathering, EntityWeatheringPolicy, StructuralProposal, WeatheringEffect};
pub use regime::{
    BatchHistory, BatchMetrics, DefaultRegimeClassifier, DynamicsRegime, RegimeClassifier,
};
pub use registry::{InfluenceKindConfig, InfluenceKindRegistry, LocusKindRegistry, PlasticityConfig};
