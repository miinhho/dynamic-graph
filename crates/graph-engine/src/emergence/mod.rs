//! Entity emergence: perspective trait and implementations.
//!
//! Per O3 in `docs/redesign.md` §8, the default perspective ships
//! without configuration: connected components in the relationship
//! graph above a minimum activity threshold, reconciled against
//! existing entity sediments using member overlap.
//!
//! The trait is `Send + Sync` so callers can store a `Box<dyn
//! EmergencePerspective>` safely alongside parallel locus processing.

mod default;

pub use default::DefaultEmergencePerspective;

use graph_core::{BatchId, EmergenceProposal};
use graph_world::{EntityStore, RelationshipStore};

/// User-replaceable hook for recognizing coherent bundles of loci.
pub trait EmergencePerspective: Send + Sync {
    /// Examine the current relationship graph (and existing sediments)
    /// and return a list of proposals for the engine to apply.
    ///
    /// The perspective is a *pure observer*: it does not mutate the
    /// store. The engine applies proposals atomically after the call.
    fn recognize(
        &self,
        relationships: &RelationshipStore,
        existing: &EntityStore,
        batch: BatchId,
    ) -> Vec<EmergenceProposal>;
}
