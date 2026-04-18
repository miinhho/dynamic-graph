use graph_core::{Endpoints, InfluenceKindId, LocusId};
use graph_world::World;

use super::shared::sort_desc_truncate;

pub fn causal_direction(world: &World, from: LocusId, to: LocusId, kind: InfluenceKindId) -> f32 {
    let ab = directed_weight(world, from, to, kind);
    let ba = directed_weight(world, to, from, kind);
    let total = ab + ba;
    if total < 1e-9 { 0.0 } else { (ab - ba) / total }
}

pub fn dominant_causes(
    world: &World,
    target: LocusId,
    kind: InfluenceKindId,
    n: usize,
) -> Vec<(LocusId, f32)> {
    let mut scored: Vec<(LocusId, f32)> = world
        .relationships()
        .iter()
        .filter(|r| r.kind == kind)
        .filter_map(|r| match r.endpoints {
            Endpoints::Directed { from, to } if to == target => Some((from, r.weight())),
            _ => None,
        })
        .collect();
    sort_desc_truncate(&mut scored, n);
    scored
}

pub fn dominant_effects(
    world: &World,
    source: LocusId,
    kind: InfluenceKindId,
    n: usize,
) -> Vec<(LocusId, f32)> {
    let mut scored: Vec<(LocusId, f32)> = world
        .relationships()
        .iter()
        .filter(|r| r.kind == kind)
        .filter_map(|r| match r.endpoints {
            Endpoints::Directed { from, to } if from == source => Some((to, r.weight())),
            _ => None,
        })
        .collect();
    sort_desc_truncate(&mut scored, n);
    scored
}

pub fn feedback_pairs(
    world: &World,
    kind: InfluenceKindId,
    min_weight: f32,
    min_balance: f32,
) -> Vec<(LocusId, LocusId, f32)> {
    use rustc_hash::{FxHashMap, FxHashSet};

    let weights: FxHashMap<(LocusId, LocusId), f32> = world
        .relationships()
        .iter()
        .filter(|r| r.kind == kind)
        .filter_map(|r| match r.endpoints {
            Endpoints::Directed { from, to } => Some(((from, to), r.weight())),
            _ => None,
        })
        .collect();

    let mut results = Vec::new();
    let mut seen: FxHashSet<(LocusId, LocusId)> = FxHashSet::default();

    for (&(from, to), &w_ab) in &weights {
        if w_ab < min_weight {
            continue;
        }
        let canonical = if from <= to { (from, to) } else { (to, from) };
        if seen.contains(&canonical) {
            continue;
        }
        let w_ba = weights.get(&(to, from)).copied().unwrap_or(0.0);
        if w_ba < min_weight {
            continue;
        }
        let max_w = w_ab.max(w_ba);
        let balance = w_ab.min(w_ba) / max_w;
        if balance >= min_balance {
            seen.insert(canonical);
            results.push((from, to, balance));
        }
    }
    results.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
    results
}

pub fn causal_in_strength(world: &World, locus: LocusId, kind: InfluenceKindId) -> f32 {
    world
        .relationships()
        .iter()
        .filter(|r| r.kind == kind)
        .filter_map(|r| match r.endpoints {
            Endpoints::Directed { to, .. } if to == locus => Some(r.weight()),
            _ => None,
        })
        .sum()
}

pub fn causal_out_strength(world: &World, locus: LocusId, kind: InfluenceKindId) -> f32 {
    world
        .relationships()
        .iter()
        .filter(|r| r.kind == kind)
        .filter_map(|r| match r.endpoints {
            Endpoints::Directed { from, .. } if from == locus => Some(r.weight()),
            _ => None,
        })
        .sum()
}

fn directed_weight(world: &World, from: LocusId, to: LocusId, kind: InfluenceKindId) -> f32 {
    world
        .relationships()
        .iter()
        .filter(|r| r.kind == kind)
        .filter_map(|r| match r.endpoints {
            Endpoints::Directed { from: f, to: t } if f == from && t == to => Some(r.weight()),
            _ => None,
        })
        .sum()
}
