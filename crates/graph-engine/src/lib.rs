//! graph-engine: substrate batch loop and emergent layers.
//!
//! See `docs/redesign.md` for the framing. Built layer by layer in
//! follow-up commits; this commit only exposes the kind registries the
//! batch loop will consume.

mod engine;
mod registry;

pub use engine::{Engine, EngineConfig, TickResult};
pub use registry::{InfluenceKindConfig, InfluenceKindRegistry, LocusKindRegistry};
