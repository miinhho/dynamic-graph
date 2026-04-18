use graph_core::{EntityId, LocusId};
use rustc_hash::FxHashMap;

pub(super) fn index_members(
    by_member: &mut FxHashMap<LocusId, Vec<EntityId>>,
    id: EntityId,
    members: &[LocusId],
) {
    for &locus in members {
        by_member.entry(locus).or_default().push(id);
    }
}

pub(super) fn deindex_members(
    by_member: &mut FxHashMap<LocusId, Vec<EntityId>>,
    id: EntityId,
    members: &[LocusId],
) {
    for locus in members {
        if let Some(ids) = by_member.get_mut(locus) {
            ids.retain(|&entity_id| entity_id != id);
        }
    }
}
