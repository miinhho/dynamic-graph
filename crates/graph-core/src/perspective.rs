//! Emergence perspective traits — the user/engine seam for entity
//! recognition.
//!
//! Per `docs/redesign.md` §3.4 and O3 (RESOLVED), the framework ships
//! a default perspective so fresh users get meaningful entities without
//! configuration. Users can replace it with a custom
//! `EmergencePerspective` for richer domain-specific recognition.
//!
//! The perspective is deliberately *stateless*: it observes current
//! state (loci + relationships + existing sediments) and returns
//! proposals. The engine applies them — maintaining atomicity,
//! minting ids, committing to the entity store.

use crate::entity::{EntityId, EntityLayer, EntitySnapshot, LifecycleCause};
use crate::ids::LocusId;
use crate::relationship::RelationshipId;

/// What an `EmergencePerspective` wants the engine to do with one
/// coherent bundle it recognized.
#[derive(Debug, Clone, PartialEq)]
pub enum EmergenceProposal {
    /// No matching existing entity — mint a new one.
    Born {
        members: Vec<LocusId>,
        member_relationships: Vec<RelationshipId>,
        coherence: f32,
        parents: Vec<EntityId>,
        cause: LifecycleCause,
    },
    /// Overlap with existing entity high enough to treat as continuation.
    DepositLayer {
        entity: EntityId,
        layer: EntityLayer,
    },
    /// One entity split into multiple offspring.
    Split {
        source: EntityId,
        offspring: Vec<(Vec<LocusId>, Vec<RelationshipId>, f32)>,
        cause: LifecycleCause,
    },
    /// Multiple entities merged.
    Merge {
        absorbed: Vec<EntityId>,
        into: EntityId,
        new_members: Vec<LocusId>,
        member_relationships: Vec<RelationshipId>,
        coherence: f32,
        cause: LifecycleCause,
    },
    /// Entity no longer coherent above threshold.
    Dormant {
        entity: EntityId,
        cause: LifecycleCause,
    },
    /// Dormant entity coherent again.
    Revive {
        entity: EntityId,
        snapshot: EntitySnapshot,
        cause: LifecycleCause,
    },
}
