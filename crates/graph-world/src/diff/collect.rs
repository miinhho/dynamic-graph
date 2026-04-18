use graph_core::{BatchId, Change, ChangeId, ChangeSubject, EntityId, LocusId, RelationshipId};
use rustc_hash::{FxHashMap, FxHashSet};

use super::RelationshipDelta;

type RelationshipActivityRange = Option<FxHashMap<RelationshipId, (f32, f32)>>;
type SubscriptionDelta = (
    Vec<(LocusId, RelationshipId)>,
    Vec<(LocusId, RelationshipId)>,
);

pub(super) fn collect_change_range(
    world: &crate::world::World,
    from: BatchId,
    to: BatchId,
) -> (Vec<ChangeId>, RelationshipActivityRange) {
    let mut change_ids = Vec::new();
    let mut rel_activity_range = None;

    for b in from.0..to.0 {
        for change in world.log().batch(BatchId(b)) {
            change_ids.push(change.id);
            record_relationship_activity(change, &mut rel_activity_range);
        }
    }

    (change_ids, rel_activity_range)
}

pub(super) fn change_id_set(change_ids: &[ChangeId]) -> FxHashSet<ChangeId> {
    change_ids.iter().copied().collect()
}

pub(super) fn collect_relationship_changes(
    world: &crate::world::World,
    in_range: &FxHashSet<ChangeId>,
) -> (Vec<RelationshipId>, Vec<RelationshipId>) {
    let mut relationships_created = Vec::new();
    let mut relationships_updated = Vec::new();

    for rel in world.relationships().iter() {
        let created_in_range = rel
            .lineage
            .created_by
            .map(|cid| in_range.contains(&cid))
            .unwrap_or(false);
        let touched_in_range = rel
            .lineage
            .last_touched_by
            .map(|cid| in_range.contains(&cid))
            .unwrap_or(false);

        if created_in_range {
            relationships_created.push(rel.id);
        } else if touched_in_range {
            relationships_updated.push(rel.id);
        }
    }

    (relationships_created, relationships_updated)
}

pub(super) fn collect_entities_changed(
    world: &crate::world::World,
    from: BatchId,
    to: BatchId,
) -> Vec<EntityId> {
    world
        .entities()
        .iter()
        .filter(|entity| {
            entity
                .layers
                .iter()
                .any(|layer| layer.batch.0 >= from.0 && layer.batch.0 < to.0)
        })
        .map(|entity| entity.id)
        .collect()
}

pub(super) fn collect_subscription_changes(
    world: &crate::world::World,
    from: BatchId,
    to: BatchId,
) -> SubscriptionDelta {
    let mut subscriptions_added = Vec::new();
    let mut subscriptions_removed = Vec::new();

    for event in world.subscriptions().events_in_range(from, to) {
        if event.subscribed {
            subscriptions_added.push((event.subscriber, event.rel_id));
        } else {
            subscriptions_removed.push((event.subscriber, event.rel_id));
        }
    }

    (subscriptions_added, subscriptions_removed)
}

pub(super) fn collect_relationship_trajectory(
    rel_activity_range: RelationshipActivityRange,
) -> (Vec<RelationshipDelta>, Vec<RelationshipDelta>) {
    let mut relationships_strengthening = Vec::new();
    let mut relationships_weakening = Vec::new();

    for (id, (activity_before, activity_after)) in rel_activity_range.unwrap_or_default() {
        let delta = RelationshipDelta {
            id,
            activity_before,
            activity_after,
        };
        if activity_after > activity_before {
            relationships_strengthening.push(delta);
        } else if activity_after < activity_before {
            relationships_weakening.push(delta);
        }
    }

    (relationships_strengthening, relationships_weakening)
}

pub(super) fn collect_pruned_relationships(
    world: &crate::world::World,
    from: BatchId,
    to: BatchId,
) -> Vec<RelationshipId> {
    world
        .pruned_log()
        .iter()
        .filter(|(_, batch)| batch.0 >= from.0 && batch.0 < to.0)
        .map(|(id, _)| *id)
        .collect()
}

fn record_relationship_activity(
    change: &Change,
    rel_activity_range: &mut RelationshipActivityRange,
) {
    if let ChangeSubject::Relationship(rel_id) = change.subject {
        let before_act = change.before.as_slice().first().copied().unwrap_or(0.0);
        let after_act = change.after.as_slice().first().copied().unwrap_or(0.0);
        rel_activity_range
            .get_or_insert_with(FxHashMap::default)
            .entry(rel_id)
            .and_modify(|range| range.1 = after_act)
            .or_insert((before_act, after_act));
    }
}
