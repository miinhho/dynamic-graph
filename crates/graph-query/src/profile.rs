//! Relationship profile: a structured view of all coupling between two loci.
//!
//! `RelationshipBundle` groups every `Relationship` that connects locus `a`
//! to locus `b` (either direction, any kind) into a single object. This
//! bridges the gap between the engine's per-kind storage and the query
//! surface's need for a multi-dimensional relationship view.
//!
//! # Usage
//!
//! ```ignore
//! let profile = graph_query::relationship_profile(&world, node_a, node_b);
//!
//! println!("net activity : {:.2}", profile.net_activity());
//! println!("dominant kind: {:?}", profile.dominant_kind());
//!
//! for (kind, activity) in profile.activity_by_kind() {
//!     println!("  kind={kind:?}  activity={activity:.3}");
//! }
//!
//! // Cosine similarity between two relationship profiles
//! let sim = profile_ab.state_similarity(&profile_ac);
//! ```

use graph_core::{BatchId, InfluenceKindId, InteractionEffect, LocusId, RelationshipId, Relationship};
use graph_world::World;

use crate::causality::changes_to_relationship_in_range;

/// All relationships between a specific pair of loci, across every kind.
///
/// Obtained via [`relationship_profile`]. The bundle is valid for the lifetime
/// of the `&World` borrow it was created from.
///
/// Directionality is preserved in the stored `Relationship` references; the
/// bundle itself is **undirected** — it collects edges in either direction.
pub struct RelationshipBundle<'w> {
    /// One of the two loci (the one passed as `a` to [`relationship_profile`]).
    pub a: LocusId,
    /// The other locus.
    pub b: LocusId,
    /// Every relationship connecting `a` and `b`, in any direction and of any kind.
    pub relationships: Vec<&'w Relationship>,
}

impl<'w> RelationshipBundle<'w> {
    /// `true` when no relationships exist between `a` and `b`.
    pub fn is_empty(&self) -> bool {
        self.relationships.is_empty()
    }

    /// Number of distinct relationship objects between `a` and `b`.
    ///
    /// One object per `(direction, kind)` pair — the same kind can appear
    /// at most twice (A→B and B→A as separate directed edges).
    pub fn len(&self) -> usize {
        self.relationships.len()
    }

    /// Sum of activity across all relationships in the bundle.
    ///
    /// Positive → net excitatory coupling; negative → net inhibitory coupling.
    pub fn net_activity(&self) -> f32 {
        self.relationships.iter().map(|r| r.activity()).sum()
    }

    /// All distinct influence kinds present in the bundle, deduplicated.
    pub fn kinds(&self) -> Vec<InfluenceKindId> {
        let mut seen = rustc_hash::FxHashSet::default();
        self.relationships
            .iter()
            .filter_map(|r| {
                if seen.insert(r.kind) { Some(r.kind) } else { None }
            })
            .collect()
    }

    /// Total activity grouped by influence kind, sorted descending by activity.
    ///
    /// When the same kind appears in both directions (A→B and B→A), their
    /// activities are summed.
    pub fn activity_by_kind(&self) -> Vec<(InfluenceKindId, f32)> {
        let mut map: rustc_hash::FxHashMap<InfluenceKindId, f32> =
            rustc_hash::FxHashMap::default();
        for rel in &self.relationships {
            *map.entry(rel.kind).or_insert(0.0) += rel.activity();
        }
        let mut pairs: Vec<_> = map.into_iter().collect();
        pairs.sort_by(|x, y| y.1.partial_cmp(&x.1).unwrap_or(std::cmp::Ordering::Equal));
        pairs
    }

    /// The influence kind with the highest total activity, or `None` when
    /// the bundle is empty.
    pub fn dominant_kind(&self) -> Option<InfluenceKindId> {
        self.activity_by_kind().into_iter().next().map(|(k, _)| k)
    }

    /// `true` when `net_activity() > 0`.
    pub fn is_excitatory(&self) -> bool {
        self.net_activity() > 0.0
    }

    /// `true` when `net_activity() < 0`.
    pub fn is_inhibitory(&self) -> bool {
        self.net_activity() < 0.0
    }

    /// All relationships in the bundle of a specific influence kind.
    pub fn of_kind(&self, kind: InfluenceKindId) -> Vec<&Relationship> {
        self.relationships.iter().copied().filter(|r| r.kind == kind).collect()
    }

    /// Net activity after applying declared cross-kind interaction effects.
    ///
    /// Works identically to [`crate::net_influence_between`] but uses the
    /// already-collected `relationships`, avoiding a second world scan.
    ///
    /// `interaction_fn(ka, kb)` returns `None` for no declared interaction
    /// (kinds are summed additively). Return `Some(InteractionEffect::Neutral)`
    /// to explicitly mark a pair as additive.
    pub fn net_activity_with_interactions<F>(&self, interaction_fn: F) -> f32
    where
        F: Fn(InfluenceKindId, InfluenceKindId) -> Option<InteractionEffect>,
    {
        use rustc_hash::{FxHashMap, FxHashSet};

        let mut by_kind: FxHashMap<InfluenceKindId, f32> = FxHashMap::default();
        for rel in &self.relationships {
            *by_kind.entry(rel.kind).or_insert(0.0) += rel.activity();
        }

        if by_kind.is_empty() {
            return 0.0;
        }

        let kinds: Vec<InfluenceKindId> = by_kind.keys().copied().collect();
        let mut merged: FxHashSet<InfluenceKindId> = FxHashSet::default();
        let mut total = 0.0f32;

        for i in 0..kinds.len() {
            for j in (i + 1)..kinds.len() {
                let ka = kinds[i];
                let kb = kinds[j];
                if let Some(effect) = interaction_fn(ka, kb) {
                    let combined = by_kind[&ka] + by_kind[&kb];
                    let adjusted = match effect {
                        InteractionEffect::Synergistic { boost } => combined * boost,
                        InteractionEffect::Antagonistic { dampen } => combined * dampen,
                        InteractionEffect::Neutral => combined,
                    };
                    total += adjusted;
                    merged.insert(ka);
                    merged.insert(kb);
                }
            }
        }

        for (kind, activity) in &by_kind {
            if !merged.contains(kind) {
                total += activity;
            }
        }

        total
    }

    /// Cosine similarity between this bundle and another, computed on a
    /// kind-indexed activity vector.
    ///
    /// Both bundles are projected onto the **union** of their kind sets. Kinds
    /// present in one but not the other contribute 0 to that bundle's vector.
    /// Returns 0.0 when either bundle is empty.
    ///
    /// This gives a direction-insensitive measure of "how similar is the
    /// multi-dimensional coupling profile between two locus pairs".
    ///
    /// Note: unlike [`StateVector::cosine_similarity`] (which truncates to the
    /// shorter vector), this method always uses the **union** of both kind sets,
    /// so missing kinds are zero-padded symmetrically.  The dimension-mismatch
    /// caveat on `StateVector::cosine_similarity` does **not** apply here.
    pub fn profile_similarity(&self, other: &RelationshipBundle<'_>) -> f32 {
        let all_kinds: Vec<InfluenceKindId> = {
            let mut set = rustc_hash::FxHashSet::default();
            for r in &self.relationships { set.insert(r.kind); }
            for r in &other.relationships { set.insert(r.kind); }
            let mut v: Vec<_> = set.into_iter().collect();
            v.sort();
            v
        };

        let vec_a: Vec<f32> = {
            let map: rustc_hash::FxHashMap<_, _> = self
                .activity_by_kind()
                .into_iter()
                .collect();
            all_kinds.iter().map(|k| *map.get(k).unwrap_or(&0.0)).collect()
        };
        let vec_b: Vec<f32> = {
            let map: rustc_hash::FxHashMap<_, _> = other
                .activity_by_kind()
                .into_iter()
                .collect();
            all_kinds.iter().map(|k| *map.get(k).unwrap_or(&0.0)).collect()
        };

        let sv_a = graph_core::StateVector::from_slice(&vec_a);
        let sv_b = graph_core::StateVector::from_slice(&vec_b);
        sv_a.cosine_similarity(&sv_b)
    }
}

impl<'w> RelationshipBundle<'w> {
    /// Cosine similarity between the **OLS activity-slope vectors** of two bundles.
    ///
    /// Where `profile_similarity` compares a *snapshot* of current activity,
    /// this method asks a different question: **"do these two locus pairs move
    /// together over time?"**
    ///
    /// # Algorithm
    ///
    /// 1. For each relationship in the bundle, collect the sequence of
    ///    `ChangeSubject::Relationship` log entries within `[from_batch, to_batch]`.
    /// 2. Fit an OLS linear regression to the activity slot of those entries
    ///    (y = activity, x = change-index within the window).  The slope is the
    ///    per-change-index rate of change.
    /// 3. Sum slopes by influence kind (A→B and B→A slopes for the same kind are
    ///    added, matching `activity_by_kind`'s semantics).
    /// 4. Project both bundles onto the **union** of their kind sets (missing
    ///    kinds get slope 0), then compute cosine similarity.
    ///
    /// Returns `0.0` when either bundle has no log entries in the range
    /// (insufficient data for any regression).
    ///
    /// # Interpretation
    ///
    /// - `+1.0` — both pairs exhibit the same directional trend in every kind.
    /// - `0.0`  — orthogonal trends (or one side has no trend data).
    /// - `-1.0` — opposing trends: one pair is strengthening where the other weakens.
    pub fn profile_trend_similarity(
        &self,
        other: &RelationshipBundle<'_>,
        world: &World,
        from_batch: BatchId,
        to_batch: BatchId,
    ) -> f32 {
        let map_a = slope_map(self, world, from_batch, to_batch);
        let map_b = slope_map(other, world, from_batch, to_batch);

        if map_a.is_empty() && map_b.is_empty() {
            return 0.0;
        }

        let all_kinds = union_sorted_map_keys(&map_a, &map_b);
        let vec_a: Vec<f32> = all_kinds.iter().map(|k| *map_a.get(k).unwrap_or(&0.0)).collect();
        let vec_b: Vec<f32> = all_kinds.iter().map(|k| *map_b.get(k).unwrap_or(&0.0)).collect();

        let sv_a = graph_core::StateVector::from_slice(&vec_a);
        let sv_b = graph_core::StateVector::from_slice(&vec_b);
        sv_a.cosine_similarity(&sv_b)
    }
}

/// Build a kind→sum-of-slopes map for all relationships in a bundle.
fn slope_map(
    bundle: &RelationshipBundle<'_>,
    world: &World,
    from_batch: BatchId,
    to_batch: BatchId,
) -> rustc_hash::FxHashMap<InfluenceKindId, f32> {
    let mut by_kind: rustc_hash::FxHashMap<InfluenceKindId, f32> =
        rustc_hash::FxHashMap::default();
    for rel in &bundle.relationships {
        if let Some(s) = ols_slope_for_rel(world, rel.id, from_batch, to_batch) {
            *by_kind.entry(rel.kind).or_insert(0.0) += s;
        }
    }
    by_kind
}

/// Return the sorted union of keys from two `FxHashMap<InfluenceKindId, _>`.
fn union_sorted_map_keys(
    a: &rustc_hash::FxHashMap<InfluenceKindId, f32>,
    b: &rustc_hash::FxHashMap<InfluenceKindId, f32>,
) -> Vec<InfluenceKindId> {
    let mut set = rustc_hash::FxHashSet::default();
    for &k in a.keys() { set.insert(k); }
    for &k in b.keys() { set.insert(k); }
    let mut v: Vec<_> = set.into_iter().collect();
    v.sort();
    v
}

fn ols_slope_for_rel(
    world: &World,
    rel: RelationshipId,
    from_batch: BatchId,
    to_batch: BatchId,
) -> Option<f32> {
    let changes = changes_to_relationship_in_range(world, rel, from_batch, to_batch);
    crate::causality::ols_activity_slope(&changes)
}

/// Collect all relationships between `a` and `b` into a [`RelationshipBundle`].
///
/// Both directions are included (A→B, B→A, and Symmetric A↔B). Loci that
/// have no mutual relationships produce an empty bundle.
///
/// ```ignore
/// let bundle = relationship_profile(&world, sender, receiver);
/// if !bundle.is_empty() {
///     println!("net coupling: {:.2}", bundle.net_activity());
/// }
/// ```
pub fn relationship_profile<'w>(world: &'w World, a: LocusId, b: LocusId) -> RelationshipBundle<'w> {
    let relationships = world.relationships_between(a, b).collect();
    RelationshipBundle { a, b, relationships }
}

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{Endpoints, InfluenceKindId, LocusId, StateVector};

    fn make_world_ab() -> (World, LocusId, LocusId) {
        let a = LocusId(0);
        let b = LocusId(1);
        let mut w = World::new();
        let k1 = InfluenceKindId(1);
        let k2 = InfluenceKindId(2);
        // A→B: k1 activity=2.0, k2 activity=-1.0
        w.add_relationship(Endpoints::directed(a, b), k1, StateVector::from_slice(&[2.0, 0.0]));
        w.add_relationship(Endpoints::directed(a, b), k2, StateVector::from_slice(&[-1.0, 0.0]));
        // B→A: k1 activity=0.5
        w.add_relationship(Endpoints::directed(b, a), k1, StateVector::from_slice(&[0.5, 0.0]));
        // Unrelated edge A→C
        w.add_relationship(
            Endpoints::directed(a, LocusId(2)),
            k1,
            StateVector::from_slice(&[3.0, 0.0]),
        );
        (w, a, b)
    }

    #[test]
    fn profile_collects_both_directions() {
        let (w, a, b) = make_world_ab();
        let p = relationship_profile(&w, a, b);
        assert_eq!(p.len(), 3); // A→B k1, A→B k2, B→A k1
    }

    #[test]
    fn profile_excludes_unrelated_edges() {
        let (w, a, b) = make_world_ab();
        let p = relationship_profile(&w, a, b);
        assert!(p.relationships.iter().all(|r| r.endpoints.involves(b)));
    }

    #[test]
    fn net_activity_sums_correctly() {
        let (w, a, b) = make_world_ab();
        let net = relationship_profile(&w, a, b).net_activity();
        assert!((net - 1.5).abs() < 1e-5, "expected 1.5, got {net}");
    }

    #[test]
    fn activity_by_kind_merges_both_directions() {
        let (w, a, b) = make_world_ab();
        let pairs = relationship_profile(&w, a, b).activity_by_kind();
        let k1_sum = pairs.iter().find(|(k, _)| *k == InfluenceKindId(1)).map(|(_, v)| *v);
        // k1: 2.0 (A→B) + 0.5 (B→A) = 2.5
        assert!((k1_sum.unwrap() - 2.5).abs() < 1e-5);
    }

    #[test]
    fn dominant_kind_is_highest_activity() {
        let (w, a, b) = make_world_ab();
        assert_eq!(relationship_profile(&w, a, b).dominant_kind(), Some(InfluenceKindId(1)));
    }

    #[test]
    fn empty_profile_when_no_edges() {
        let w = World::new();
        let p = relationship_profile(&w, LocusId(0), LocusId(1));
        assert!(p.is_empty());
        assert_eq!(p.net_activity(), 0.0);
        assert!(p.dominant_kind().is_none());
    }

    #[test]
    fn is_excitatory_inhibitory() {
        let (w, a, b) = make_world_ab();
        let p = relationship_profile(&w, a, b);
        assert!(p.is_excitatory());
        assert!(!p.is_inhibitory());
    }

    #[test]
    fn profile_similarity_identical_profiles() {
        let (w, a, b) = make_world_ab();
        let p = relationship_profile(&w, a, b);
        // Similarity with itself = 1.0
        assert!((p.profile_similarity(&p) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn profile_similarity_orthogonal_kinds() {
        // Profile AB has only k1; profile CD has only k2 → orthogonal → 0.0
        let k1 = InfluenceKindId(1);
        let k2 = InfluenceKindId(2);
        let mut w = World::new();
        w.add_relationship(
            Endpoints::directed(LocusId(0), LocusId(1)),
            k1,
            StateVector::from_slice(&[1.0, 0.0]),
        );
        w.add_relationship(
            Endpoints::directed(LocusId(2), LocusId(3)),
            k2,
            StateVector::from_slice(&[1.0, 0.0]),
        );
        let pab = relationship_profile(&w, LocusId(0), LocusId(1));
        let pcd = relationship_profile(&w, LocusId(2), LocusId(3));
        assert!((pab.profile_similarity(&pcd)).abs() < 1e-5);
    }
}
