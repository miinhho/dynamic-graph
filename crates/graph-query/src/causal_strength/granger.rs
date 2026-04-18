use graph_core::{InfluenceKindId, LocusId};
use graph_world::World;

use super::shared::{neighbors_of_kind, sort_desc_truncate};

pub fn granger_score(
    world: &World,
    from: LocusId,
    to: LocusId,
    kind: InfluenceKindId,
    lag_batches: u64,
) -> f32 {
    let mut b_batches: Vec<u64> = world
        .changes_to_locus(to)
        .filter(|c| c.kind == kind)
        .map(|c| c.batch.0)
        .collect();
    if b_batches.is_empty() {
        return 0.0;
    }
    b_batches.sort_unstable();

    let a_changes: Vec<u64> = world
        .changes_to_locus(from)
        .filter(|c| c.kind == kind)
        .map(|c| c.batch.0)
        .collect();
    let n = a_changes.len();
    if n == 0 {
        return 0.0;
    }

    let mut co_count = 0usize;
    for &t in &a_changes {
        let lo = t.saturating_add(1);
        let hi = t.saturating_add(lag_batches);
        let idx = b_batches.partition_point(|&x| x < lo);
        if idx < b_batches.len() && b_batches[idx] <= hi {
            co_count += 1;
        }
    }

    co_count as f32 / n as f32
}

pub fn granger_dominant_causes(
    world: &World,
    target: LocusId,
    kind: InfluenceKindId,
    lag_batches: u64,
    n: usize,
) -> Vec<(LocusId, f32)> {
    let candidates = neighbors_of_kind(world, target, kind);
    let mut scored: Vec<(LocusId, f32)> = candidates
        .into_iter()
        .map(|src| (src, granger_score(world, src, target, kind, lag_batches)))
        .filter(|(_, s)| *s > 0.0)
        .collect();
    sort_desc_truncate(&mut scored, n);
    scored
}

pub fn granger_dominant_effects(
    world: &World,
    source: LocusId,
    kind: InfluenceKindId,
    lag_batches: u64,
    n: usize,
) -> Vec<(LocusId, f32)> {
    let candidates = neighbors_of_kind(world, source, kind);
    let mut scored: Vec<(LocusId, f32)> = candidates
        .into_iter()
        .map(|tgt| (tgt, granger_score(world, source, tgt, kind, lag_batches)))
        .filter(|(_, s)| *s > 0.0)
        .collect();
    sort_desc_truncate(&mut scored, n);
    scored
}
