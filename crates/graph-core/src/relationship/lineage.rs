use smallvec::SmallVec;

use crate::ids::{BatchId, ChangeId, InfluenceKindId};

use super::{KindObservation, RelationshipLineage};

pub(super) fn new_emerged(
    change_id: ChangeId,
    kind: InfluenceKindId,
    batch: BatchId,
) -> RelationshipLineage {
    let mut kinds_observed = SmallVec::new();
    kinds_observed.push(KindObservation::once(kind, batch));
    RelationshipLineage {
        created_by: Some(change_id),
        last_touched_by: Some(change_id),
        change_count: 1,
        kinds_observed,
    }
}

pub(super) fn new_synthetic(kind: InfluenceKindId) -> RelationshipLineage {
    let mut kinds_observed = SmallVec::new();
    kinds_observed.push(KindObservation::synthetic(kind));
    RelationshipLineage {
        created_by: None,
        last_touched_by: None,
        change_count: 1,
        kinds_observed,
    }
}

pub(super) fn empty() -> RelationshipLineage {
    RelationshipLineage {
        created_by: None,
        last_touched_by: None,
        change_count: 0,
        kinds_observed: SmallVec::new(),
    }
}

pub(super) fn observe_kind(
    kinds_observed: &mut SmallVec<[KindObservation; 2]>,
    kind: InfluenceKindId,
    batch: BatchId,
) {
    if let Some(obs) = kinds_observed.iter_mut().find(|o| o.kind == kind) {
        obs.touch_count += 1;
        obs.last_batch = batch;
    } else {
        kinds_observed.push(KindObservation::once(kind, batch));
    }
}

pub(super) fn dominant_flow_kind(
    kinds_observed: &SmallVec<[KindObservation; 2]>,
) -> Option<InfluenceKindId> {
    kinds_observed
        .iter()
        .max_by(|a, b| {
            a.touch_count
                .cmp(&b.touch_count)
                .then_with(|| b.kind.0.cmp(&a.kind.0))
        })
        .map(|obs| obs.kind)
}

pub(super) fn touch_count_for(
    kinds_observed: &SmallVec<[KindObservation; 2]>,
    kind: InfluenceKindId,
) -> u64 {
    kinds_observed
        .iter()
        .find(|o| o.kind == kind)
        .map(|o| o.touch_count)
        .unwrap_or(0)
}

pub(super) fn has_seen_kind(
    kinds_observed: &SmallVec<[KindObservation; 2]>,
    kind: InfluenceKindId,
) -> bool {
    kinds_observed.iter().any(|o| o.kind == kind)
}
