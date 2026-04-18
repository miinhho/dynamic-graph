use graph_core::{BatchId, Change, ChangeId, ChangeSubject, LocusId, RelationshipId};
use rustc_hash::FxHashMap;

use super::ChangeLog;

pub(super) struct IndexUpdates {
    pub(super) batch: BatchId,
    pub(super) batch_ids: Vec<ChangeId>,
    pub(super) locus_groups: FxHashMap<LocusId, Vec<ChangeId>>,
    pub(super) relationship_groups: FxHashMap<RelationshipId, Vec<ChangeId>>,
}

pub(super) fn build_index_updates(changes: &[Change]) -> IndexUpdates {
    let mut locus_groups: FxHashMap<LocusId, Vec<ChangeId>> =
        FxHashMap::with_capacity_and_hasher(16, Default::default());
    let mut relationship_groups: FxHashMap<RelationshipId, Vec<ChangeId>> =
        FxHashMap::with_capacity_and_hasher(4, Default::default());
    let mut batch_ids = Vec::with_capacity(changes.len());

    for change in changes {
        batch_ids.push(change.id);
        match change.subject {
            ChangeSubject::Locus(id) => locus_groups.entry(id).or_default().push(change.id),
            ChangeSubject::Relationship(id) => {
                relationship_groups.entry(id).or_default().push(change.id)
            }
        }
    }

    IndexUpdates {
        batch: changes[0].batch,
        batch_ids,
        locus_groups,
        relationship_groups,
    }
}

pub(super) fn apply_index_updates(log: &mut ChangeLog, updates: IndexUpdates) {
    extend_batch_index(&mut log.by_batch, updates.batch, &updates.batch_ids);
    merge_locus_groups(&mut log.by_locus, updates.locus_groups);
    merge_relationship_groups(&mut log.by_relationship, updates.relationship_groups);
}

fn extend_batch_index(
    by_batch: &mut FxHashMap<BatchId, Vec<ChangeId>>,
    batch: BatchId,
    batch_ids: &[ChangeId],
) {
    let batch_vec = by_batch.entry(batch).or_default();
    batch_vec.reserve(batch_ids.len());
    batch_vec.extend(batch_ids.iter().copied());
}

fn merge_locus_groups(
    by_locus: &mut FxHashMap<LocusId, Vec<ChangeId>>,
    groups: FxHashMap<LocusId, Vec<ChangeId>>,
) {
    for (locus_id, ids) in groups {
        by_locus.entry(locus_id).or_default().extend(ids);
    }
}

fn merge_relationship_groups(
    by_relationship: &mut FxHashMap<RelationshipId, Vec<ChangeId>>,
    groups: FxHashMap<RelationshipId, Vec<ChangeId>>,
) {
    for (rel_id, ids) in groups {
        by_relationship.entry(rel_id).or_default().extend(ids);
    }
}
