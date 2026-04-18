use graph_core::{BatchId, InfluenceKindId};
use graph_world::World;
use rustc_hash::{FxHashMap, FxHashSet};

use super::RelationshipBundle;

pub(super) fn activity_map(bundle: &RelationshipBundle<'_>) -> FxHashMap<InfluenceKindId, f32> {
    let mut by_kind: FxHashMap<InfluenceKindId, f32> = FxHashMap::default();
    for relationship in &bundle.relationships {
        *by_kind.entry(relationship.kind).or_insert(0.0) += relationship.activity();
    }
    by_kind
}

pub(super) fn trend_map(
    bundle: &RelationshipBundle<'_>,
    world: &World,
    from_batch: BatchId,
    to_batch: BatchId,
) -> FxHashMap<InfluenceKindId, f32> {
    let mut by_kind: FxHashMap<InfluenceKindId, f32> = FxHashMap::default();
    for relationship in &bundle.relationships {
        let changes = crate::causality::changes_to_relationship_in_range(
            world,
            relationship.id,
            from_batch,
            to_batch,
        );
        if let Some(slope) = crate::causality::ols_activity_slope(&changes) {
            *by_kind.entry(relationship.kind).or_insert(0.0) += slope;
        }
    }
    by_kind
}

pub(super) fn union_kinds(
    left: &FxHashMap<InfluenceKindId, f32>,
    right: &FxHashMap<InfluenceKindId, f32>,
) -> Vec<InfluenceKindId> {
    let mut kinds = FxHashSet::default();
    for &kind in left.keys() {
        kinds.insert(kind);
    }
    for &kind in right.keys() {
        kinds.insert(kind);
    }
    let mut ordered: Vec<_> = kinds.into_iter().collect();
    ordered.sort();
    ordered
}
