use graph_core::{BatchId, InfluenceKindId, LocusId, RelationshipId};

use super::{SubscriptionEvent, SubscriptionStore};

pub(super) fn subscribe_at(
    store: &mut SubscriptionStore,
    subscriber: LocusId,
    rel_id: RelationshipId,
    batch: Option<BatchId>,
) {
    let inserted = store
        .by_relationship
        .entry(rel_id)
        .or_default()
        .insert(subscriber);
    store.by_locus.entry(subscriber).or_default().insert(rel_id);
    if inserted {
        store.generation += 1;
        store.total_count += 1;
        record_audit_event(store, batch, subscriber, rel_id, true);
    }
}

pub(super) fn unsubscribe_at(
    store: &mut SubscriptionStore,
    subscriber: LocusId,
    rel_id: RelationshipId,
    batch: Option<BatchId>,
) {
    let removed = store
        .by_relationship
        .get_mut(&rel_id)
        .map(|subscribers| subscribers.remove(&subscriber))
        .unwrap_or(false);
    if let Some(relationships) = store.by_locus.get_mut(&subscriber) {
        relationships.remove(&rel_id);
    }
    if removed {
        store.generation += 1;
        store.total_count -= 1;
        record_audit_event(store, batch, subscriber, rel_id, false);
    }
}

pub(super) fn subscribe_to_kind(
    store: &mut SubscriptionStore,
    subscriber: LocusId,
    kind: InfluenceKindId,
) {
    let inserted = store.by_kind.entry(kind).or_default().insert(subscriber);
    if inserted {
        store.generation += 1;
        store
            .kinds_by_subscriber
            .entry(subscriber)
            .or_default()
            .insert(kind);
    }
}

pub(super) fn unsubscribe_from_kind(
    store: &mut SubscriptionStore,
    subscriber: LocusId,
    kind: InfluenceKindId,
) {
    let removed = store
        .by_kind
        .get_mut(&kind)
        .map(|subscribers| subscribers.remove(&subscriber))
        .unwrap_or(false);
    if removed {
        store.generation += 1;
        if let Some(kinds) = store.kinds_by_subscriber.get_mut(&subscriber) {
            kinds.remove(&kind);
        }
    }
}

pub(super) fn subscribe_to_anchor_kind(
    store: &mut SubscriptionStore,
    subscriber: LocusId,
    anchor: LocusId,
    kind: InfluenceKindId,
) {
    let key = (anchor, kind);
    let inserted = store
        .by_anchor_kind
        .entry(key)
        .or_default()
        .insert(subscriber);
    if inserted {
        store.generation += 1;
        store
            .anchor_kinds_by_subscriber
            .entry(subscriber)
            .or_default()
            .insert(key);
        store
            .kinds_by_anchor
            .entry(anchor)
            .or_default()
            .insert(kind);
    }
}

pub(super) fn unsubscribe_from_anchor_kind(
    store: &mut SubscriptionStore,
    subscriber: LocusId,
    anchor: LocusId,
    kind: InfluenceKindId,
) {
    let key = (anchor, kind);
    let removed = store
        .by_anchor_kind
        .get_mut(&key)
        .map(|subscribers| subscribers.remove(&subscriber))
        .unwrap_or(false);
    if removed {
        store.generation += 1;
        if let Some(keys) = store.anchor_kinds_by_subscriber.get_mut(&subscriber) {
            keys.remove(&key);
        }
        let anchor_set_empty = store
            .by_anchor_kind
            .get(&key)
            .map(|subscribers| subscribers.is_empty())
            .unwrap_or(true);
        if anchor_set_empty && let Some(kinds) = store.kinds_by_anchor.get_mut(&anchor) {
            kinds.remove(&kind);
        }
    }
}

fn record_audit_event(
    store: &mut SubscriptionStore,
    batch: Option<BatchId>,
    subscriber: LocusId,
    rel_id: RelationshipId,
    subscribed: bool,
) {
    if let Some(batch) = batch {
        store
            .audit_log
            .entry(batch.0)
            .or_default()
            .push(SubscriptionEvent {
                batch,
                subscriber,
                rel_id,
                subscribed,
            });
    }
}
