//! Behavioral equivalence of loci via bounded bisimulation.
//!
//! See `docs/coalgebra.md` for the categorical framing. This module
//! ships the runtime-side primitive that actually answers
//! "are these two loci interchangeable?":
//!
//! - [`behavioral_partition`] — partition every locus in the world into
//!   `k`-bisimulation classes under a chosen `LocusEncoder` and
//!   `EdgeEncoder`. Classes coarsen as `rounds` decreases (depth-0 only
//!   compares the encoder output; depth-`k` agrees iff all `k`-hop
//!   neighborhoods agree).
//!
//! - [`behavior_signature`] — the single locus's color after `k` rounds.
//!   Use when you need a stable hashable label for one locus rather
//!   than the full partition.
//!
//! - [`behaviorally_equivalent`] — convenience wrapper that checks two
//!   loci for `k`-bisimilarity by comparing their signatures.
//!
//! ## Algorithm
//!
//! Weisfeiler-Lehman color refinement:
//!
//! ```text
//! color_0(v) = encode_locus(v)
//! color_{k+1}(v) = fold(color_k(v), sorted [
//!                       (encode_edge(e), color_k(other(e, v)), direction(e, v))
//!                       for e in edges_touching(v)
//!                  ])
//! ```
//!
//! Two loci are `k`-bisimilar (under the chosen encoders) iff
//! `color_k` agrees on them. The procedure is well-known to converge to
//! the largest bisimulation in `O(|V|)` rounds for finite carriers, and
//! it is exactly the algorithm used by Hopcroft-Paige-Tarjan partition
//! refinement up to a constant factor. Early termination on fixpoint is
//! built in.
//!
//! ## Cost
//!
//! `O(rounds × (|V| + |E| log Δ))` where `Δ` is the maximum degree
//! (the `log Δ` is the per-locus sort of the neighborhood signature).
//! Hash collisions in `BehaviorColor` are possible in principle but
//! cosmetically unlikely with the FNV-1a folding used here; partition
//! quality is unaffected as long as the encoder itself is collision-free.

use graph_core::{
    BehaviorColor, EdgeDirection, EdgeEncoder, KindOnlyEdgeEncoder, KindOnlyEncoder, LocusEncoder,
    LocusId, fold_color,
};
use graph_world::World;
use rustc_hash::FxHashMap;

/// Parameters controlling the refinement run. Use [`BisimOptions::default`]
/// for kind-only encodings (purely topological equivalence) and override
/// individual fields when needed.
#[derive(Debug, Clone)]
pub struct BisimOptions<L: LocusEncoder, E: EdgeEncoder> {
    /// Number of refinement rounds. `0` returns the encoder's seed
    /// coloring directly. Larger values progressively distinguish loci
    /// by deeper neighborhood structure; the algorithm short-circuits
    /// when the partition reaches a fixpoint.
    pub rounds: u32,
    pub locus_encoder: L,
    pub edge_encoder: E,
}

impl Default for BisimOptions<KindOnlyEncoder, KindOnlyEdgeEncoder> {
    fn default() -> Self {
        Self {
            rounds: 4,
            locus_encoder: KindOnlyEncoder,
            edge_encoder: KindOnlyEdgeEncoder,
        }
    }
}

/// Compute the `rounds`-bisimulation partition of every locus in `world`.
///
/// Returns a `Vec` of equivalence classes (each a `Vec<LocusId>`).
/// Classes are sorted internally by `LocusId` and the outer `Vec` is
/// sorted by the smallest member of each class, so the output is
/// deterministic and stable across runs.
///
/// Loci with no relationships still receive a color — they fall into a
/// class determined entirely by their encoder seed.
pub fn behavioral_partition<L: LocusEncoder, E: EdgeEncoder>(
    world: &World,
    opts: &BisimOptions<L, E>,
) -> Vec<Vec<LocusId>> {
    let colors = compute_colors(world, opts);
    let mut buckets: FxHashMap<BehaviorColor, Vec<LocusId>> = FxHashMap::default();
    for (id, c) in colors {
        buckets.entry(c).or_default().push(id);
    }
    let mut classes: Vec<Vec<LocusId>> = buckets
        .into_values()
        .map(|mut v| {
            v.sort();
            v
        })
        .collect();
    // Sort classes by their smallest member for stable output.
    classes.sort_by_key(|c| c.first().copied().unwrap_or(LocusId(u64::MAX)));
    classes
}

/// Compute the bisimulation color of `locus` after `rounds` of
/// refinement. Returns `None` if `locus` is not present in `world`.
pub fn behavior_signature<L: LocusEncoder, E: EdgeEncoder>(
    world: &World,
    locus: LocusId,
    opts: &BisimOptions<L, E>,
) -> Option<BehaviorColor> {
    let colors = compute_colors(world, opts);
    colors.get(&locus).copied()
}

/// Returns `true` iff `a` and `b` have the same color after `rounds`
/// refinement steps. Returns `false` if either locus is absent.
pub fn behaviorally_equivalent<L: LocusEncoder, E: EdgeEncoder>(
    world: &World,
    a: LocusId,
    b: LocusId,
    opts: &BisimOptions<L, E>,
) -> bool {
    let colors = compute_colors(world, opts);
    match (colors.get(&a), colors.get(&b)) {
        (Some(ca), Some(cb)) => ca == cb,
        _ => false,
    }
}

// ── Final-coalgebra projection ────────────────────────────────────────────

/// Iterate refinement to fixpoint and return the **final-coalgebra
/// projection** of every locus: the unique image of each state in the
/// largest bisimulation quotient.
///
/// Concretely, this runs `behavioral_partition` with `rounds` set high
/// enough that further refinement does not change the partition (the
/// algorithm shorts-circuits at fixpoint internally). The resulting
/// `BehaviorColor` per locus is a canonical identity tag: two loci
/// share a color iff they are bisimilar **at every depth**.
///
/// Use this as the definitive answer to "are these the same?" — for
/// example, as the equivalence test in entity merge candidate
/// detection. Note that "every depth" here means up to the number of
/// distinct loci; for a finite world this is reached in at most
/// `|loci|` rounds, usually far fewer.
///
/// Same encoder semantics as `behavioral_partition` apply: the
/// projection is *relative* to the chosen `LocusEncoder` and
/// `EdgeEncoder`.
pub fn behavior_fixpoint<L: LocusEncoder, E: EdgeEncoder>(
    world: &World,
    locus: LocusId,
    locus_encoder: L,
    edge_encoder: E,
) -> Option<BehaviorColor> {
    let opts = BisimOptions {
        rounds: world.loci().iter().count() as u32,
        locus_encoder,
        edge_encoder,
    };
    let colors = compute_colors(world, &opts);
    colors.get(&locus).copied()
}

/// Return the final-coalgebra partition of all loci — i.e. the largest
/// bisimulation quotient under the chosen encoders. Equivalent to
/// `behavioral_partition` with rounds set so high that the partition
/// is guaranteed to stabilize.
pub fn behavioral_partition_fixpoint<L: LocusEncoder, E: EdgeEncoder>(
    world: &World,
    locus_encoder: L,
    edge_encoder: E,
) -> Vec<Vec<LocusId>> {
    let opts = BisimOptions {
        rounds: world.loci().iter().count() as u32,
        locus_encoder,
        edge_encoder,
    };
    behavioral_partition(world, &opts)
}

// ── Core refinement loop ──────────────────────────────────────────────────

fn compute_colors<L: LocusEncoder, E: EdgeEncoder>(
    world: &World,
    opts: &BisimOptions<L, E>,
) -> FxHashMap<LocusId, BehaviorColor> {
    // Round 0: seed coloring from the locus encoder.
    let mut current: FxHashMap<LocusId, BehaviorColor> = world
        .loci()
        .iter()
        .map(|l| (l.id, opts.locus_encoder.encode_locus(l)))
        .collect();

    if opts.rounds == 0 || current.is_empty() {
        return current;
    }

    let mut next: FxHashMap<LocusId, BehaviorColor> =
        FxHashMap::with_capacity_and_hasher(current.len(), Default::default());
    let mut neighborhood: Vec<(BehaviorColor, BehaviorColor, EdgeDirection)> = Vec::new();

    for _ in 0..opts.rounds {
        next.clear();
        for locus in world.loci().iter() {
            neighborhood.clear();
            for rel in world.relationships_for_locus(locus.id) {
                let other = rel.endpoints.other_than(locus.id);
                let dir = match EdgeDirection::of(&rel.endpoints, locus.id) {
                    Some(d) => d,
                    None => continue, // shouldn't happen, but be defensive
                };
                let edge_color = opts.edge_encoder.encode_edge(rel);
                let neighbor_color = current.get(&other).copied().unwrap_or(0);
                neighborhood.push((edge_color, neighbor_color, dir));
            }
            // Sort to make the multiset signature canonical.
            neighborhood.sort();
            let own = current.get(&locus.id).copied().unwrap_or(0);
            let new_color = fold_color(own, own, &neighborhood);
            next.insert(locus.id, new_color);
        }

        // Fixpoint detection: if the *partition* (not the raw colors)
        // didn't change, we can stop. The partition stabilizes when the
        // map old_color -> new_color is functional, which is equivalent
        // to the multiset of new_colors having the same equivalence
        // structure as the multiset of old_colors. Cheap proxy: same
        // mapping table.
        if partitions_equal(&current, &next) {
            break;
        }
        std::mem::swap(&mut current, &mut next);
    }

    current
}

/// `true` iff the two colorings induce the same equivalence partition.
/// We test this by checking that each `(old, new)` pair seen is
/// consistent — i.e. no `old` color maps to two distinct `new` colors
/// and no `new` color is reached from two distinct `old` colors.
fn partitions_equal(
    a: &FxHashMap<LocusId, BehaviorColor>,
    b: &FxHashMap<LocusId, BehaviorColor>,
) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut a_to_b: FxHashMap<BehaviorColor, BehaviorColor> = FxHashMap::default();
    let mut b_to_a: FxHashMap<BehaviorColor, BehaviorColor> = FxHashMap::default();
    for (id, ca) in a {
        let cb = match b.get(id) {
            Some(c) => *c,
            None => return false,
        };
        if let Some(prev) = a_to_b.insert(*ca, cb)
            && prev != cb
        {
            return false;
        }
        if let Some(prev) = b_to_a.insert(cb, *ca)
            && prev != *ca
        {
            return false;
        }
    }
    true
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{
        BatchId, Endpoints, InfluenceKindId, KindObservation, Locus, LocusKindId, Relationship,
        RelationshipKindId, RelationshipLineage, StateVector,
    };

    fn make_locus(world: &mut World, id: u64, kind: u64) {
        world.insert_locus(Locus::new(
            LocusId(id),
            LocusKindId(kind),
            StateVector::from_slice(&[0.0]),
        ));
    }

    fn link(world: &mut World, from: u64, to: u64, kind: u64) {
        let rid = world.relationships_mut().mint_id();
        let kind_id: RelationshipKindId = InfluenceKindId(kind);
        world.relationships_mut().insert(Relationship {
            id: rid,
            kind: kind_id,
            endpoints: Endpoints::Directed {
                from: LocusId(from),
                to: LocusId(to),
            },
            state: StateVector::from_slice(&[1.0, 0.0]),
            lineage: RelationshipLineage {
                created_by: None,
                last_touched_by: None,
                change_count: 1,
                kinds_observed: smallvec::smallvec![KindObservation::synthetic(kind_id)],
            },
            created_batch: BatchId(0),
            last_decayed_batch: 0,
            metadata: None,
        });
    }

    #[test]
    fn isolated_loci_of_same_kind_are_zero_bisimilar() {
        let mut w = World::new();
        make_locus(&mut w, 1, 7);
        make_locus(&mut w, 2, 7);
        make_locus(&mut w, 3, 8);
        let opts = BisimOptions {
            rounds: 0,
            locus_encoder: KindOnlyEncoder,
            edge_encoder: KindOnlyEdgeEncoder,
        };
        assert!(behaviorally_equivalent(&w, LocusId(1), LocusId(2), &opts));
        assert!(!behaviorally_equivalent(&w, LocusId(1), LocusId(3), &opts));
    }

    #[test]
    fn matching_chains_collapse_to_three_classes() {
        // Two parallel chains of length 3, same kind throughout. After
        // refinement: heads form one class, middles another, tails a
        // third. Six loci → three classes, each of size two.
        let mut w = World::new();
        for i in 1..=6 {
            make_locus(&mut w, i, 1);
        }
        link(&mut w, 1, 2, 1);
        link(&mut w, 2, 3, 1);
        link(&mut w, 4, 5, 1);
        link(&mut w, 5, 6, 1);
        let opts = BisimOptions::default();
        let classes = behavioral_partition(&w, &opts);
        assert_eq!(classes.len(), 3, "got {classes:?}");
        for c in &classes {
            assert_eq!(c.len(), 2);
        }
    }

    #[test]
    fn asymmetric_neighborhood_separates_loci() {
        // Locus 1 and 2 have the same kind, but 1 has an outgoing edge
        // to a kind-9 locus while 2 has none. After 1 round, they
        // should sit in separate classes.
        let mut w = World::new();
        make_locus(&mut w, 1, 1);
        make_locus(&mut w, 2, 1);
        make_locus(&mut w, 3, 9);
        link(&mut w, 1, 3, 1);
        let opts = BisimOptions {
            rounds: 1,
            locus_encoder: KindOnlyEncoder,
            edge_encoder: KindOnlyEdgeEncoder,
        };
        assert!(!behaviorally_equivalent(&w, LocusId(1), LocusId(2), &opts));
    }

    #[test]
    fn missing_locus_signature_is_none() {
        let w = World::new();
        let opts = BisimOptions::default();
        assert_eq!(
            behavior_signature(&w, LocusId(99), &opts),
            None,
            "no loci registered → no signature"
        );
    }

    #[test]
    fn fixpoint_terminates_before_max_rounds() {
        // Two singletons: a fixpoint is reached after round 1, regardless
        // of how many rounds we ask for. We can't directly observe early
        // termination, but we can confirm behavior matches the 1-round
        // case for any larger round count.
        let mut w = World::new();
        make_locus(&mut w, 1, 1);
        make_locus(&mut w, 2, 1);
        let opts1 = BisimOptions {
            rounds: 1,
            locus_encoder: KindOnlyEncoder,
            edge_encoder: KindOnlyEdgeEncoder,
        };
        let opts100 = BisimOptions {
            rounds: 100,
            locus_encoder: KindOnlyEncoder,
            edge_encoder: KindOnlyEdgeEncoder,
        };
        assert_eq!(
            behavioral_partition(&w, &opts1),
            behavioral_partition(&w, &opts100),
        );
    }

    #[test]
    fn fixpoint_partition_matches_high_round_count() {
        // For a small world the fixpoint partition should equal the
        // `rounds = many` partition.
        let mut w = World::new();
        for i in 1..=4 {
            make_locus(&mut w, i, 1);
        }
        link(&mut w, 1, 2, 1);
        link(&mut w, 3, 4, 1);
        let high = BisimOptions {
            rounds: 50,
            locus_encoder: KindOnlyEncoder,
            edge_encoder: KindOnlyEdgeEncoder,
        };
        let p_high = behavioral_partition(&w, &high);
        let p_fix = behavioral_partition_fixpoint(&w, KindOnlyEncoder, KindOnlyEdgeEncoder);
        assert_eq!(p_high, p_fix);
    }

    #[test]
    fn fixpoint_signature_stable_across_calls() {
        let mut w = World::new();
        make_locus(&mut w, 1, 1);
        make_locus(&mut w, 2, 1);
        link(&mut w, 1, 2, 1);
        let s1 = behavior_fixpoint(&w, LocusId(1), KindOnlyEncoder, KindOnlyEdgeEncoder);
        let s2 = behavior_fixpoint(&w, LocusId(1), KindOnlyEncoder, KindOnlyEdgeEncoder);
        assert!(s1.is_some());
        assert_eq!(s1, s2, "fixpoint signature must be deterministic");
    }

    #[test]
    fn directed_edges_distinguish_source_from_target() {
        // 1 → 2 (both same kind). After 1 round, the source and the
        // target should land in different classes because their edge
        // direction differs.
        let mut w = World::new();
        make_locus(&mut w, 1, 1);
        make_locus(&mut w, 2, 1);
        link(&mut w, 1, 2, 1);
        let opts = BisimOptions {
            rounds: 1,
            locus_encoder: KindOnlyEncoder,
            edge_encoder: KindOnlyEdgeEncoder,
        };
        assert!(!behaviorally_equivalent(&w, LocusId(1), LocusId(2), &opts));
    }
}
