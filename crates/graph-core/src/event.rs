//! World events — lightweight signals emitted during world mutations.
//!
//! Events are collected during `apply_proposals`, `flush_relationship_decay`,
//! and regime transitions, then returned to the caller via `StepObservation`.
//! They are pure data — no callbacks, no subscriptions, no allocation beyond
//! the `Vec<WorldEvent>` itself.

use crate::entity::EntityId;
use crate::ids::{BatchId, ChangeId, InfluenceKindId, LocusId, LocusKindId};
use crate::relationship::RelationshipId;

/// A discrete event emitted by a world mutation.
#[derive(Debug, Clone, PartialEq)]
pub enum WorldEvent {
    /// A new entity was recognized and born.
    EntityBorn {
        entity: EntityId,
        batch: BatchId,
        member_count: usize,
    },
    /// An active entity became dormant.
    EntityDormant { entity: EntityId, batch: BatchId },
    /// A dormant entity was revived.
    EntityRevived { entity: EntityId, batch: BatchId },
    /// An entity split into offspring.
    EntitySplit {
        source: EntityId,
        offspring: Vec<EntityId>,
        batch: BatchId,
    },
    /// Multiple entities merged into one.
    EntityMerged {
        absorbed: Vec<EntityId>,
        into: EntityId,
        batch: BatchId,
    },
    /// An entity's coherence changed significantly.
    CoherenceShift {
        entity: EntityId,
        from: f32,
        to: f32,
        batch: BatchId,
    },
    /// A new relationship auto-emerged from co-occurrence of two loci.
    RelationshipEmerged {
        relationship: RelationshipId,
        from: LocusId,
        to: LocusId,
        kind: InfluenceKindId,
        /// The `ChangeId` that triggered emergence — the change that first
        /// observed causal flow from `from` to `to` under this `kind`.
        trigger_change_id: ChangeId,
    },
    /// A relationship was auto-pruned due to low activity.
    RelationshipPruned { relationship: RelationshipId },
    /// A relationship of `kind` emerged between loci whose kinds are not
    /// listed in `applies_between` for that influence kind.
    /// Soft violation — the relationship is still created; this is advisory.
    SchemaViolation {
        relationship: RelationshipId,
        kind: InfluenceKindId,
        from_locus_kind: LocusKindId,
        to_locus_kind: LocusKindId,
    },
    /// The dynamics regime shifted.
    RegimeShift {
        from: super::regime_tag::RegimeTag,
        to: super::regime_tag::RegimeTag,
    },
}
