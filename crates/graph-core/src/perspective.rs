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

use crate::entity::{EntityId, EntityLayer, EntitySnapshot};
use crate::ids::LocusId;

/// What an `EmergencePerspective` wants the engine to do with one
/// coherent bundle it recognized.
#[derive(Debug, Clone, PartialEq)]
pub enum EmergenceProposal {
    /// No matching existing entity — mint a new one.
    Born {
        members: Vec<LocusId>,
        coherence: f32,
        parents: Vec<EntityId>,
    },
    /// Overlap with existing entity high enough to treat as continuation.
    DepositLayer {
        entity: EntityId,
        layer: EntityLayer,
    },
    /// One entity split into multiple offspring.
    Split {
        source: EntityId,
        offspring: Vec<(Vec<LocusId>, f32)>,
    },
    /// Multiple entities merged.
    Merge {
        absorbed: Vec<EntityId>,
        into: EntityId,
        new_members: Vec<LocusId>,
        coherence: f32,
    },
    /// Entity no longer coherent above threshold.
    Dormant { entity: EntityId },
    /// Dormant entity coherent again.
    Revive {
        entity: EntityId,
        snapshot: EntitySnapshot,
    },
}
