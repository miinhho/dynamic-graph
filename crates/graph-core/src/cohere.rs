//! Layer 4: cohere — clusters of relationships/entities under a
//! user-supplied perspective.
//!
//! Per `docs/redesign.md` §3.6 a cohere is *not* a primitive. It is a
//! view produced by a `CoherePerspective` applied to the current state
//! of the world. Multiple perspectives can coexist, each producing a
//! different set of coheres over the same underlying graph.
//!
//! A cohere could represent a community, a context, a coordinated
//! group, or any other "cluster" concept the user's domain requires.

use crate::entity::EntityId;
use crate::relationship::RelationshipId;

/// Stable identity of a cohere cluster.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CohereId(pub u64);

/// What members a cohere aggregates. A perspective can cluster entities,
/// relationships, or a mix — it depends on the domain.
#[derive(Debug, Clone, PartialEq)]
pub enum CohereMembers {
    Entities(Vec<EntityId>),
    Relationships(Vec<RelationshipId>),
    Mixed {
        entities: Vec<EntityId>,
        relationships: Vec<RelationshipId>,
    },
}

impl CohereMembers {
    pub fn entity_count(&self) -> usize {
        match self {
            CohereMembers::Entities(ids) => ids.len(),
            CohereMembers::Mixed { entities, .. } => entities.len(),
            CohereMembers::Relationships(_) => 0,
        }
    }

    pub fn relationship_count(&self) -> usize {
        match self {
            CohereMembers::Relationships(ids) => ids.len(),
            CohereMembers::Mixed { relationships, .. } => relationships.len(),
            CohereMembers::Entities(_) => 0,
        }
    }
}

/// A cohere cluster produced by one application of a `CoherePerspective`.
#[derive(Debug, Clone, PartialEq)]
pub struct Cohere {
    pub id: CohereId,
    /// Membership as judged by the producing perspective.
    pub members: CohereMembers,
    /// Cluster-internal cohesion score, computed by the perspective.
    /// Semantics are perspective-defined; the engine treats it opaquely.
    pub strength: f32,
}
