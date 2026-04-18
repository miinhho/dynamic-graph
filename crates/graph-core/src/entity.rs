//! Layer 3: entity — the second emergent layer (sedimentary).
//!
//! Entities are *not* registered by the user. They are recognized by
//! the engine when the relationship graph exhibits coherent bundles.
//! Per `docs/redesign.md` §3.4 the key commitments are:
//!
//! - **Sedimentary**: each significant change deposits a new `EntityLayer`
//!   on the entity's stack. Old layers weather but the identity is
//!   permanent.
//! - **Never deleted**: dormant entities remain in the store.
//! - **Lineage**: parent/child references preserved beyond layer
//!   weathering.

use crate::ids::{BatchId, LocusId};
use crate::relationship::RelationshipId;

mod layers;

/// Why a lifecycle transition happened. Provides a causal link from
/// entity-layer events back to the lower-layer evidence that triggered them.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum LifecycleCause {
    /// No specific cause recorded (e.g. external stimulus, or cause
    /// tracking was not enabled).
    #[default]
    Unspecified,
    /// Entity formed from these relationships being above threshold.
    RelationshipCluster {
        /// The key relationships that formed the cluster.
        key_relationships: Vec<RelationshipId>,
    },
    /// Entity went dormant because these relationships decayed.
    RelationshipDecay {
        /// Relationships whose activity fell below threshold.
        decayed_relationships: Vec<RelationshipId>,
    },
    /// Entity split because the component graph separated.
    ComponentSplit {
        /// Bridge relationships that were too weak to hold the entity
        /// together.
        weak_bridges: Vec<RelationshipId>,
    },
    /// Entity absorbed others via merge (survivor's perspective).
    MergedFrom { absorbed: Vec<EntityId> },
    /// Entity was absorbed into another (absorbed entity's perspective).
    MergedInto { survivor: EntityId },
}

/// Stable identity of an entity. Immutable across its entire sediment
/// stack from birth to dormancy.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct EntityId(pub u64);

/// Whether the entity is currently coherent enough to accumulate new
/// layers. Dormant entities remain in the store forever.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum EntityStatus {
    /// Currently recognized by the perspective; layers are being
    /// deposited.
    Active,
    /// No longer coherent above threshold; no new layers are deposited,
    /// but the entity (and its remaining layers) are preserved.
    Dormant,
}

/// Snapshot of which loci and relationships constituted the entity at
/// a given layer. The "current" entity is the snapshot at the top of
/// its sediment stack.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct EntitySnapshot {
    pub members: Vec<LocusId>,
    pub member_relationships: Vec<RelationshipId>,
    /// Coherence score computed by the `EmergencePerspective`. Semantics
    /// are perspective-defined; the engine treats it opaquely except to
    /// compare against the dormancy threshold.
    pub coherence: f32,
}

impl Default for EntitySnapshot {
    fn default() -> Self {
        Self::empty()
    }
}

impl EntitySnapshot {
    pub fn empty() -> Self {
        layers::empty_snapshot()
    }
}

/// What caused a new layer to be deposited. The engine uses this to
/// resist weathering on significant transitions (Born, Split, Merged).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum LayerTransition {
    /// First layer — entity was just born.
    Born,
    /// Membership changed; records which loci were added/removed.
    MembershipDelta {
        added: Vec<LocusId>,
        removed: Vec<LocusId>,
    },
    /// Coherence score moved significantly.
    CoherenceShift { from: f32, to: f32 },
    /// Entity split; offspring entity ids are listed.
    Split { offspring: Vec<EntityId> },
    /// Entity merged others into itself; absorbed ids are listed.
    Merged { absorbed: Vec<EntityId> },
    /// Entity went dormant.
    BecameDormant,
    /// Entity revived from dormancy.
    Revived,
}

impl LayerTransition {
    /// True for transitions that the default weathering policy exempts
    /// from removal (see `docs/redesign.md` §3.5).
    pub fn is_significant(&self) -> bool {
        layers::is_significant_transition(self)
    }
}

/// Detail level of an entity layer after weathering.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum CompressionLevel {
    /// Recent layer — full detail preserved.
    Full,
    /// Mid-age — member lists and relationship refs dropped; coherence
    /// score and member count retained.
    Compressed {
        coherence: f32,
        member_count: u32,
        transition_kind: CompressedTransition,
    },
    /// Very old — only a skeleton remains. Preserved only if the
    /// transition was significant; otherwise the layer is removed
    /// entirely by the weathering policy.
    Skeleton {
        coherence: f32,
        member_count: u32,
        transition_kind: CompressedTransition,
    },
}

/// A lossless tag for what type of transition happened, used inside
/// compressed and skeleton layers where the full `LayerTransition`
/// variants are too large to keep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum CompressedTransition {
    Born,
    MembershipDelta,
    CoherenceShift,
    Split,
    Merged,
    BecameDormant,
    Revived,
}

impl From<&LayerTransition> for CompressedTransition {
    fn from(t: &LayerTransition) -> Self {
        layers::compressed_transition(t)
    }
}

/// A single sediment layer on an entity's stack.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct EntityLayer {
    /// The batch during which this layer was deposited.
    pub batch: BatchId,
    /// What the entity looked like at this layer (only meaningful when
    /// `compression == Full`; otherwise the snapshot is stripped by
    /// weathering).
    pub snapshot: Option<EntitySnapshot>,
    pub transition: LayerTransition,
    pub compression: CompressionLevel,
    /// Why this transition happened — links back to the lower-layer
    /// evidence. Default: `Unspecified`.
    pub cause: LifecycleCause,
}

impl EntityLayer {
    pub fn new(batch: BatchId, snapshot: EntitySnapshot, transition: LayerTransition) -> Self {
        layers::new_entity_layer(batch, snapshot, transition)
    }

    pub fn with_cause(mut self, cause: LifecycleCause) -> Self {
        self.cause = cause;
        self
    }
}

/// Long-lived parent/child references. Outlasts the layer detail;
/// preserved by the lineage-tree memory layer per `docs/redesign.md`
/// §3.5.
#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct EntityLineage {
    pub parents: Vec<EntityId>,
    pub children: Vec<EntityId>,
}

/// The entity record in the store.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Entity {
    /// Stable id — never changes.
    pub id: EntityId,
    /// Most recent full snapshot. Kept separate from the layer stack so
    /// callers don't need to walk down to find it.
    pub current: EntitySnapshot,
    /// Sediment layers, oldest first (index 0), newest last.
    pub layers: Vec<EntityLayer>,
    pub lineage: EntityLineage,
    pub status: EntityStatus,
}

impl Entity {
    /// Construct a newly born entity.
    pub fn born(id: EntityId, batch: BatchId, snapshot: EntitySnapshot) -> Self {
        layers::born_entity(id, batch, snapshot)
    }

    /// Deposit a new layer on top of the sediment stack, updating
    /// `current` to reflect the transition.
    pub fn deposit(
        &mut self,
        batch: BatchId,
        snapshot: EntitySnapshot,
        transition: LayerTransition,
    ) {
        layers::deposit_layer(self, batch, snapshot, transition);
    }

    /// Total number of layers deposited (including the birth layer).
    pub fn layer_count(&self) -> usize {
        self.layers.len()
    }
}
