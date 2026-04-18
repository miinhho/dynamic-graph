use graph_core::{BatchId, Change, ChangeId, ChangeSubject, InfluenceKindId, LocusId, TrimSummary};
use rustc_hash::FxHashMap;

use super::ChangeLog;

type TrimLocusStats = FxHashMap<LocusId, (u32, Vec<f32>, Vec<f32>, Vec<InfluenceKindId>, BatchId)>;

pub(super) fn trim_before_batch(log: &mut ChangeLog, retain_from_batch: BatchId) -> usize {
    let split = log.changes.partition_point(|c| c.batch < retain_from_batch);
    if split == 0 {
        return 0;
    }

    record_trim_summaries(log, split, retain_from_batch);

    if split == log.changes.len() {
        clear_all_indices(log);
        return split;
    }
    trim_indices_after_split(log, split, retain_from_batch);
    log.changes.drain(..split).count()
}

fn record_trim_summaries(log: &mut ChangeLog, split: usize, retain_from_batch: BatchId) {
    let locus_stats = collect_trim_locus_stats(&log.changes[..split], &mut log.trimmed_id_to_locus);
    append_trim_summaries(&mut log.summaries, locus_stats, retain_from_batch);
}

fn clear_all_indices(log: &mut ChangeLog) {
    log.by_locus.clear();
    log.by_relationship.clear();
    log.by_batch.clear();
    log.changes.clear();
}

fn trim_indices_after_split(log: &mut ChangeLog, split: usize, retain_from_batch: BatchId) {
    let first_kept = log.changes[split].id;
    trim_id_index_map(&mut log.by_locus, first_kept);
    trim_id_index_map(&mut log.by_relationship, first_kept);
    log.by_batch.retain(|&batch, _| batch >= retain_from_batch);
}

fn collect_trim_locus_stats(
    trimmed_changes: &[Change],
    trimmed_id_to_locus: &mut FxHashMap<ChangeId, LocusId>,
) -> TrimLocusStats {
    let mut locus_stats = FxHashMap::default();
    for change in trimmed_changes {
        let ChangeSubject::Locus(locus) = change.subject else {
            continue;
        };
        trimmed_id_to_locus.insert(change.id, locus);
        record_trimmed_locus_change(&mut locus_stats, locus, change);
    }
    locus_stats
}

fn record_trimmed_locus_change(locus_stats: &mut TrimLocusStats, locus: LocusId, change: &Change) {
    let entry = locus_stats.entry(locus).or_insert_with(|| {
        let dim = change
            .after
            .as_slice()
            .len()
            .max(change.before.as_slice().len());
        (0, vec![0.0; dim], vec![0.0; dim], Vec::new(), change.batch)
    });
    entry.0 += 1;
    accumulate_state_sums(entry, change);
    if !entry.3.contains(&change.kind) {
        entry.3.push(change.kind);
    }
    if change.batch < entry.4 {
        entry.4 = change.batch;
    }
}

fn accumulate_state_sums(
    entry: &mut (u32, Vec<f32>, Vec<f32>, Vec<InfluenceKindId>, BatchId),
    change: &Change,
) {
    let after = change.after.as_slice();
    let before = change.before.as_slice();
    let dim = entry.1.len();
    for i in 0..dim {
        entry.1[i] += after.get(i).copied().unwrap_or(0.0);
        entry.2[i] += before.get(i).copied().unwrap_or(0.0);
    }
}

fn append_trim_summaries(
    summaries: &mut FxHashMap<LocusId, Vec<TrimSummary>>,
    locus_stats: TrimLocusStats,
    retain_from_batch: BatchId,
) {
    for (locus, (count, sum_after, sum_before, kinds, batch_from)) in locus_stats {
        let delta: Vec<f32> = sum_after
            .iter()
            .zip(sum_before.iter())
            .map(|(a, b)| a - b)
            .collect();
        let summary = TrimSummary {
            locus,
            batch_from,
            batch_to: retain_from_batch,
            change_count: count,
            net_delta_state: graph_core::StateVector::from_slice(&delta),
            kinds_observed: kinds,
        };
        summaries.entry(locus).or_default().push(summary);
    }
}

fn trim_id_index_map<K>(index: &mut FxHashMap<K, Vec<ChangeId>>, first_kept: ChangeId)
where
    K: std::hash::Hash + Eq,
{
    for ids in index.values_mut() {
        let remove = ids.partition_point(|&id| id < first_kept);
        ids.drain(..remove);
    }
}
