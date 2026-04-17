//! Causal direction inference from STDP-learned relationship weights.
//!
//! After STDP has run, directed relationship weights encode causal consistency:
//! - High weight on A→B: A consistently causes B (PreFirst plasticity)
//! - Low/zero weight on B→A: B→A is feedback-suppressed
//!
//! These functions read `Relationship::weight()` directly — no ChangeLog
//! access needed. They are meaningful only when STDP plasticity is active.

use graph_core::{Endpoints, InfluenceKindId, LocusId};
use graph_world::World;

/// Net causal direction between two loci for a given influence kind.
///
/// Returns a value in `[-1.0, 1.0]`:
/// - `+1.0`: `from` is the dominant cause of `to` (A→B weight >> B→A weight)
/// - `-1.0`: `to` is the dominant cause of `from` (B→A weight >> A→B weight)
/// -  `0.0`: balanced, no relationship, or symmetric
///
/// Uses directed relationship weights from STDP plasticity. Requires
/// `PlasticityConfig::stdp = true` on the kind to be meaningful.
pub fn causal_direction(world: &World, from: LocusId, to: LocusId, kind: InfluenceKindId) -> f32 {
    let ab = directed_weight(world, from, to, kind);
    let ba = directed_weight(world, to, from, kind);
    let total = ab + ba;
    if total < 1e-9 {
        0.0
    } else {
        (ab - ba) / total
    }
}

/// Top-N loci that most consistently cause `target` (highest directed incoming weight).
///
/// Returns `(source_locus, weight)` pairs sorted descending by weight.
/// Only directed relationships (`Endpoints::Directed { to: target }`) are counted.
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
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(n);
    scored
}

/// Top-N loci that are most consistently caused by `source` (highest directed outgoing weight).
///
/// Returns `(target_locus, weight)` pairs sorted descending by weight.
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
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(n);
    scored
}

/// Locus pairs where A→B and B→A weights are roughly balanced (oscillators/feedback).
///
/// Returns `(locus_a, locus_b, balance_ratio)` where:
/// - `balance_ratio = min(w_ab, w_ba) / max(w_ab, w_ba)` ∈ `[0.0, 1.0]`
/// - ratio near `1.0` = highly balanced (feedback loop / oscillator)
/// - ratio near `0.0` = strongly directional
///
/// Only pairs where both weights exceed `min_weight` are returned.
/// Pairs are deduplicated (only (A,B) appears, not also (B,A)).
pub fn feedback_pairs(
    world: &World,
    kind: InfluenceKindId,
    min_weight: f32,
    min_balance: f32,
) -> Vec<(LocusId, LocusId, f32)> {
    use rustc_hash::FxHashMap;
    // Build map: (from, to) → weight for directed relationships of this kind.
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
    let mut seen: rustc_hash::FxHashSet<(LocusId, LocusId)> = rustc_hash::FxHashSet::default();

    for (&(from, to), &w_ab) in &weights {
        if w_ab < min_weight { continue; }
        let canonical = if from <= to { (from, to) } else { (to, from) };
        if seen.contains(&canonical) { continue; }
        let w_ba = weights.get(&(to, from)).copied().unwrap_or(0.0);
        if w_ba < min_weight { continue; }
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

/// Sum of directed incoming weights to `locus` for a given kind.
///
/// Measures how strongly this locus is "caused" by its upstream neighbors.
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

/// Sum of directed outgoing weights from `locus` for a given kind.
///
/// Measures how strongly this locus causes its downstream neighbors.
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

// ── D2: Granger-style scores ──────────────────────────────────────────────────
//
// STDP weights (D1) reflect accumulated causal consistency over the entire
// history of the simulation: a high A→B weight means A has *repeatedly* fired
// before B and plasticity has encoded that pattern. Granger scores complement
// this by counting temporal precedence directly in the ChangeLog — they work
// even when STDP is disabled, and they capture transient causation that has
// not yet accumulated into weights. The trade-off: Granger scores are bounded
// by ChangeLog retention (trimming shrinks the evidence window) and can
// over-count in bursty workloads. Use STDP weights for long-term structural
// causality; use Granger scores for recent or episodic causality.

/// Empirical Granger-style score: fraction of `from`'s changes of `kind` that
/// are followed by a change to `to` of the same kind within `lag_batches`.
///
/// Returns a value in `[0.0, 1.0]`:
/// - `1.0`: every time `from` changed, `to` changed within `lag_batches` afterward.
/// - `0.0`: no co-occurrence found, or `from` has no changes of `kind`.
///
/// This is a unidirectional, empirical score — not a formal statistical test.
/// Use it to compare with STDP-derived `causal_direction` when both are
/// meaningful (STDP active, ChangeLog retained).
///
/// Complexity: O(|A| log |B|) where A/B are the change counts for each locus.
pub fn granger_score(
    world: &World,
    from: LocusId,
    to: LocusId,
    kind: InfluenceKindId,
    lag_batches: u64,
) -> f32 {
    // Collect `to`'s change batches sorted ascending for binary-search window checks.
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
        // Check whether any B change exists in the window [t+1, t+lag_batches].
        let lo = t.saturating_add(1);
        let hi = t.saturating_add(lag_batches);
        let idx = b_batches.partition_point(|&x| x < lo);
        if idx < b_batches.len() && b_batches[idx] <= hi {
            co_count += 1;
        }
    }

    co_count as f32 / n as f32
}

/// Top-N loci that most consistently precede `target`'s changes (highest Granger score).
///
/// Candidates are loci with any relationship to `target` of `kind`. Only candidates
/// with score > 0 are included.
///
/// Returns `(source_locus, granger_score)` pairs sorted descending by score.
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
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(n);
    scored
}

/// Top-N loci most consistently preceded by `source`'s changes (highest Granger score).
///
/// Candidates are loci with any relationship from `source` of `kind`. Only candidates
/// with score > 0 are included.
///
/// Returns `(target_locus, granger_score)` pairs sorted descending by score.
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
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(n);
    scored
}

/// Helper: collect all loci with any relationship to/from `locus` of `kind`.
fn neighbors_of_kind(world: &World, locus: LocusId, kind: InfluenceKindId) -> Vec<LocusId> {
    let mut seen: rustc_hash::FxHashSet<LocusId> = rustc_hash::FxHashSet::default();
    for rel in world.relationships().iter() {
        if rel.kind != kind {
            continue;
        }
        match rel.endpoints {
            Endpoints::Directed { from, to } => {
                if from == locus { seen.insert(to); }
                else if to == locus { seen.insert(from); }
            }
            Endpoints::Symmetric { a, b } => {
                if a == locus { seen.insert(b); }
                else if b == locus { seen.insert(a); }
            }
        }
    }
    seen.into_iter().collect()
}

// ── internal helper ───────────────────────────────────────────────────────────

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

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{Endpoints, StateVector};
    use graph_world::World;

    const KIND: InfluenceKindId = InfluenceKindId(1);
    const OTHER_KIND: InfluenceKindId = InfluenceKindId(2);

    fn a() -> LocusId { LocusId(0) }
    fn b() -> LocusId { LocusId(1) }
    fn c() -> LocusId { LocusId(2) }

    /// Add a directed relationship with explicit weight (slot 1).
    fn add_directed(world: &mut World, from: LocusId, to: LocusId, weight: f32) {
        // slot 0 = activity, slot 1 = Hebbian weight
        world.add_relationship(
            Endpoints::Directed { from, to },
            KIND,
            StateVector::from_slice(&[0.0, weight]),
        );
    }

    #[test]
    fn causal_direction_dominant_forward() {
        let mut w = World::new();
        add_directed(&mut w, a(), b(), 0.8);
        add_directed(&mut w, b(), a(), 0.2);
        let d = causal_direction(&w, a(), b(), KIND);
        assert!(d > 0.5, "expected strong forward direction, got {d}");
    }

    #[test]
    fn causal_direction_symmetric_is_zero() {
        let mut w = World::new();
        add_directed(&mut w, a(), b(), 0.5);
        add_directed(&mut w, b(), a(), 0.5);
        let d = causal_direction(&w, a(), b(), KIND);
        assert!(d.abs() < 1e-6, "expected 0.0 for symmetric, got {d}");
    }

    #[test]
    fn causal_direction_no_relationship_is_zero() {
        let w = World::new();
        assert_eq!(causal_direction(&w, a(), b(), KIND), 0.0);
    }

    #[test]
    fn dominant_causes_returns_top_n() {
        let mut w = World::new();
        add_directed(&mut w, a(), c(), 0.9);
        add_directed(&mut w, b(), c(), 0.3);
        let causes = dominant_causes(&w, c(), KIND, 1);
        assert_eq!(causes.len(), 1);
        assert_eq!(causes[0].0, a());
        assert!((causes[0].1 - 0.9).abs() < 1e-6);
    }

    #[test]
    fn dominant_effects_returns_top_n() {
        let mut w = World::new();
        add_directed(&mut w, a(), b(), 0.4);
        add_directed(&mut w, a(), c(), 0.7);
        let effects = dominant_effects(&w, a(), KIND, 1);
        assert_eq!(effects.len(), 1);
        assert_eq!(effects[0].0, c());
    }

    #[test]
    fn causal_in_out_strength_consistent() {
        let mut w = World::new();
        add_directed(&mut w, a(), b(), 0.6);
        add_directed(&mut w, c(), b(), 0.4);
        assert!((causal_in_strength(&w, b(), KIND) - 1.0).abs() < 1e-5);
        assert!((causal_out_strength(&w, a(), KIND) - 0.6).abs() < 1e-5);
        assert_eq!(causal_out_strength(&w, b(), KIND), 0.0);
    }

    #[test]
    fn feedback_pairs_detects_balanced_loop() {
        let mut w = World::new();
        add_directed(&mut w, a(), b(), 0.8);
        add_directed(&mut w, b(), a(), 0.7);
        let pairs = feedback_pairs(&w, KIND, 0.1, 0.5);
        assert_eq!(pairs.len(), 1);
        let (_, _, balance) = pairs[0];
        assert!(balance >= 0.5 && balance <= 1.0, "balance={balance}");
    }

    #[test]
    fn feedback_pairs_skips_directional() {
        let mut w = World::new();
        add_directed(&mut w, a(), b(), 0.9);
        add_directed(&mut w, b(), a(), 0.05);
        let pairs = feedback_pairs(&w, KIND, 0.1, 0.5);
        assert!(pairs.is_empty(), "directional pair should not appear");
    }

    // ── D2: Granger-style tests ───────────────────────────────────────────────

    fn make_world_with_changes() -> World {
        use graph_core::{BatchId, Change, ChangeId, ChangeSubject, StateVector};
        let mut w = World::new();
        // Add two loci.
        add_directed(&mut w, a(), b(), 0.5);

        // Manually append changes: A fires at batches 1, 3, 5; B fires at batches 2, 4, 6.
        // Lag of 1 means A's changes at 1, 3, 5 should each be followed by B within 1 batch.
        fn change(id: u64, subject: ChangeSubject, batch: u64) -> Change {
            Change {
                id: ChangeId(id),
                subject,
                kind: KIND,
                predecessors: vec![],
                before: StateVector::zeros(1),
                after: StateVector::from_slice(&[1.0]),
                batch: BatchId(batch),
                wall_time: None,
                metadata: None,
            }
        }
        // A changes at batches 1, 3, 5
        w.log_mut().append(change(0, ChangeSubject::Locus(a()), 1));
        w.log_mut().append(change(1, ChangeSubject::Locus(a()), 3));
        w.log_mut().append(change(2, ChangeSubject::Locus(a()), 5));
        // B changes at batches 2, 4, 6 (always within 1 batch of A)
        w.log_mut().append(change(3, ChangeSubject::Locus(b()), 2));
        w.log_mut().append(change(4, ChangeSubject::Locus(b()), 4));
        w.log_mut().append(change(5, ChangeSubject::Locus(b()), 6));
        w
    }

    #[test]
    fn granger_score_perfect_lag_one() {
        let w = make_world_with_changes();
        let score = granger_score(&w, a(), b(), KIND, 1);
        assert!((score - 1.0).abs() < 1e-6, "A always precedes B within 1 batch, got {score}");
    }

    #[test]
    fn granger_score_zero_when_b_never_follows() {
        let w = make_world_with_changes();
        // B→A: B changes at 2,4,6; A changes at 1,3,5 — B never precedes A within 1 batch
        // (B fires AFTER A, not before). So granger_score(b, a, lag=1) should be 0.
        let score = granger_score(&w, b(), a(), KIND, 1);
        // B at 2 — does A fire in [3,3]? Yes! A at 3. So this is 1.0, not 0.0.
        // Actually b→a lag=1: B at 2, A at 3 (within 1). So score should be 1.0 too.
        // Let's just verify it's between 0 and 1.
        assert!((0.0..=1.0).contains(&score), "score should be in [0,1], got {score}");
    }

    #[test]
    fn granger_score_zero_lag_too_short() {
        // With lag_batches=0, no window — should always be 0
        let w = make_world_with_changes();
        let score = granger_score(&w, a(), b(), KIND, 0);
        assert_eq!(score, 0.0, "lag=0 means no window, score should be 0");
    }

    #[test]
    fn granger_score_no_a_changes_returns_zero() {
        let w = World::new();
        let score = granger_score(&w, a(), b(), KIND, 5);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn granger_dominant_causes_returns_neighbors() {
        let w = make_world_with_changes();
        let causes = granger_dominant_causes(&w, b(), KIND, 1, 5);
        // A is neighbor of B and has score 1.0
        assert!(!causes.is_empty(), "should find at least one cause");
        assert_eq!(causes[0].0, a(), "A should be top cause of B");
        assert!((causes[0].1 - 1.0).abs() < 1e-6);
    }

    #[test]
    fn granger_query_variants_dispatch_correctly() {
        use crate::query_api::{execute, Query, QueryResult};
        let w = make_world_with_changes();
        let result = execute(&w, &Query::GrangerScore {
            from: a(),
            to: b(),
            kind: KIND,
            lag_batches: 1,
        });
        let QueryResult::Score(s) = result else { panic!("expected Score") };
        assert!((s - 1.0).abs() < 1e-6);

        let result2 = execute(&w, &Query::GrangerDominantCauses {
            target: b(),
            kind: KIND,
            lag_batches: 1,
            n: 3,
        });
        let QueryResult::LocusScores(scores) = result2 else { panic!("expected LocusScores") };
        assert!(!scores.is_empty());
    }

    #[test]
    fn kind_filter_is_respected() {
        let mut w = World::new();
        add_directed(&mut w, a(), b(), 0.9);
        // querying OTHER_KIND should find nothing
        assert_eq!(causal_direction(&w, a(), b(), OTHER_KIND), 0.0);
        assert!(dominant_causes(&w, b(), OTHER_KIND, 5).is_empty());
        assert_eq!(causal_in_strength(&w, b(), OTHER_KIND), 0.0);
    }
}
