use crate::ids::BatchId;

use super::{
    CompressedTransition, CompressionLevel, Entity, EntityId, EntityLayer, EntityLineage,
    EntitySnapshot, EntityStatus, LayerTransition, LifecycleCause,
};

pub(super) fn empty_snapshot() -> EntitySnapshot {
    EntitySnapshot {
        members: Vec::new(),
        member_relationships: Vec::new(),
        coherence: 0.0,
    }
}

pub(super) fn is_significant_transition(transition: &LayerTransition) -> bool {
    matches!(
        transition,
        LayerTransition::Born | LayerTransition::Split { .. } | LayerTransition::Merged { .. }
    )
}

pub(super) fn compressed_transition(transition: &LayerTransition) -> CompressedTransition {
    match transition {
        LayerTransition::Born => CompressedTransition::Born,
        LayerTransition::MembershipDelta { .. } => CompressedTransition::MembershipDelta,
        LayerTransition::CoherenceShift { .. } => CompressedTransition::CoherenceShift,
        LayerTransition::Split { .. } => CompressedTransition::Split,
        LayerTransition::Merged { .. } => CompressedTransition::Merged,
        LayerTransition::BecameDormant => CompressedTransition::BecameDormant,
        LayerTransition::Revived => CompressedTransition::Revived,
    }
}

pub(super) fn new_entity_layer(
    batch: BatchId,
    snapshot: EntitySnapshot,
    transition: LayerTransition,
) -> EntityLayer {
    EntityLayer {
        batch,
        snapshot: Some(snapshot),
        transition,
        compression: CompressionLevel::Full,
        cause: LifecycleCause::Unspecified,
    }
}

pub(super) fn born_entity(id: EntityId, batch: BatchId, snapshot: EntitySnapshot) -> Entity {
    Entity {
        id,
        current: snapshot.clone(),
        layers: vec![new_entity_layer(batch, snapshot, LayerTransition::Born)],
        lineage: EntityLineage::default(),
        status: EntityStatus::Active,
    }
}

pub(super) fn deposit_layer(
    entity: &mut Entity,
    batch: BatchId,
    snapshot: EntitySnapshot,
    transition: LayerTransition,
) {
    entity.current = snapshot.clone();
    entity
        .layers
        .push(new_entity_layer(batch, snapshot, transition));
}
