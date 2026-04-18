use graph_core::{InfluenceKindId, LocusId, RelationshipId};
use rustc_hash::{FxHashMap, FxHashSet};

pub(super) struct SpecificCleanup {
    pub(super) changed: bool,
    pub(super) removed_count: usize,
}

pub(super) fn remove_specific_for_locus(
    by_locus: &mut FxHashMap<LocusId, FxHashSet<RelationshipId>>,
    by_relationship: &mut FxHashMap<RelationshipId, FxHashSet<LocusId>>,
    locus: LocusId,
) -> SpecificCleanup {
    let Some(relationships) = by_locus.remove(&locus) else {
        return SpecificCleanup {
            changed: false,
            removed_count: 0,
        };
    };

    let removed_count = relationships.len();
    for rel_id in relationships {
        if let Some(subscribers) = by_relationship.get_mut(&rel_id) {
            subscribers.remove(&locus);
        }
    }

    SpecificCleanup {
        changed: removed_count > 0,
        removed_count,
    }
}

pub(super) fn remove_kind_scopes_for_locus(
    kinds_by_subscriber: &mut FxHashMap<LocusId, FxHashSet<InfluenceKindId>>,
    by_kind: &mut FxHashMap<InfluenceKindId, FxHashSet<LocusId>>,
    locus: LocusId,
) -> bool {
    let Some(kinds) = kinds_by_subscriber.remove(&locus) else {
        return false;
    };

    let changed = !kinds.is_empty();
    for kind in kinds {
        if let Some(subscribers) = by_kind.get_mut(&kind) {
            subscribers.remove(&locus);
        }
    }
    changed
}

pub(super) fn remove_anchor_kind_scopes_for_locus(
    anchor_kinds_by_subscriber: &mut FxHashMap<LocusId, FxHashSet<(LocusId, InfluenceKindId)>>,
    by_anchor_kind: &mut FxHashMap<(LocusId, InfluenceKindId), FxHashSet<LocusId>>,
    kinds_by_anchor: &mut FxHashMap<LocusId, FxHashSet<InfluenceKindId>>,
    locus: LocusId,
) -> bool {
    let Some(keys) = anchor_kinds_by_subscriber.remove(&locus) else {
        return false;
    };

    let changed = !keys.is_empty();
    for key in keys {
        remove_anchor_kind_subscription(by_anchor_kind, kinds_by_anchor, key, locus);
    }
    changed
}

pub(super) fn remove_anchor_locus_scopes(
    by_anchor_kind: &mut FxHashMap<(LocusId, InfluenceKindId), FxHashSet<LocusId>>,
    anchor_kinds_by_subscriber: &mut FxHashMap<LocusId, FxHashSet<(LocusId, InfluenceKindId)>>,
    kinds_by_anchor: &mut FxHashMap<LocusId, FxHashSet<InfluenceKindId>>,
    anchor: LocusId,
) -> bool {
    let Some(kinds) = kinds_by_anchor.remove(&anchor) else {
        return false;
    };
    if kinds.is_empty() {
        return false;
    }

    for kind in kinds {
        let key = (anchor, kind);
        if let Some(subscribers) = by_anchor_kind.remove(&key) {
            remove_anchor_key_from_subscribers(anchor_kinds_by_subscriber, &subscribers, key);
        }
    }
    true
}

fn remove_anchor_kind_subscription(
    by_anchor_kind: &mut FxHashMap<(LocusId, InfluenceKindId), FxHashSet<LocusId>>,
    kinds_by_anchor: &mut FxHashMap<LocusId, FxHashSet<InfluenceKindId>>,
    key: (LocusId, InfluenceKindId),
    locus: LocusId,
) {
    if let Some(subscribers) = by_anchor_kind.get_mut(&key) {
        subscribers.remove(&locus);
    }

    if by_anchor_kind
        .get(&key)
        .map(|s| s.is_empty())
        .unwrap_or(true)
    {
        remove_anchor_kind(kinds_by_anchor, key);
    }
}

fn remove_anchor_kind(
    kinds_by_anchor: &mut FxHashMap<LocusId, FxHashSet<InfluenceKindId>>,
    (anchor, kind): (LocusId, InfluenceKindId),
) {
    if let Some(kinds) = kinds_by_anchor.get_mut(&anchor) {
        kinds.remove(&kind);
    }
}

fn remove_anchor_key_from_subscribers(
    anchor_kinds_by_subscriber: &mut FxHashMap<LocusId, FxHashSet<(LocusId, InfluenceKindId)>>,
    subscribers: &FxHashSet<LocusId>,
    key: (LocusId, InfluenceKindId),
) {
    for subscriber in subscribers {
        if let Some(set) = anchor_kinds_by_subscriber.get_mut(subscriber) {
            set.remove(&key);
        }
    }
}
