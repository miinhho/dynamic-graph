//! Layer-wise tension: run boundary analysis per `RelationshipKindId`.
//!
//! In a multi-kind world, different influence kinds represent different social
//! or operational layers (e.g. "trust", "information-sharing", "authority").
//! The overall tension score mixes all layers together, which can obscure
//! per-layer divergence.
//!
//! `layer_tension` computes a [`LayerReport`] that breaks down confirmed /
//! ghost / shadow counts and the tension score for each distinct
//! `RelationshipKindId` present in the dynamic world.
//!
//! ## Matching
//!
//! A declared fact is matched against dynamic relationships **of a specific
//! kind** when querying per-layer. Facts declared without a kind restriction
//! (i.e. all facts in `SchemaWorld`, which carry a `DeclaredRelKind` string
//! predicate) are matched against the kind-filtered relationship set. The
//! caller supplies a `kind_map` that maps `DeclaredRelKind` predicates to
//! `RelationshipKindId`s — only mapped predicates participate in per-layer
//! analysis.

use rustc_hash::{FxHashMap, FxHashSet};

use graph_core::InfluenceKindId;
use graph_schema::{DeclaredFact, DeclaredRelKind, SchemaWorld};
use graph_world::World;

use crate::analysis::{SignalMode, signal};

/// Tension breakdown for a single dynamic layer (RelationshipKindId).
#[derive(Debug, Clone)]
pub struct LayerTension {
    pub kind: InfluenceKindId,
    pub confirmed: usize,
    pub ghost: usize,
    pub shadow: usize,
    /// Divergence score for this layer: `(ghost + shadow) / total.max(1)`.
    pub tension: f32,
}

impl LayerTension {
    pub fn total(&self) -> usize {
        self.confirmed + self.ghost + self.shadow
    }
}

/// Per-layer breakdown of boundary tension.
#[derive(Debug, Clone)]
pub struct LayerReport {
    /// One entry per distinct `RelationshipKindId` present in the dynamic
    /// world, sorted descending by tension score.
    pub layers: Vec<LayerTension>,
    /// The layer with the highest tension (most divergence).
    pub most_divergent: Option<InfluenceKindId>,
    /// The layer with the lowest tension (most aligned).
    pub most_aligned: Option<InfluenceKindId>,
}

/// Compute per-layer boundary tension.
///
/// `kind_map` maps `DeclaredRelKind` string predicates to the
/// `RelationshipKindId` they correspond to in the dynamic world. Only facts
/// whose predicate appears in `kind_map` are included in per-layer analysis;
/// all other facts are ignored.
///
/// Relationships whose kind is not in `kind_map` values appear in the
/// `shadow` count of their layer (they have no declared counterpart).
///
/// `threshold` and `mode` are forwarded to `analyze_boundary_with_mode` for
/// each kind-filtered sub-world view.
pub fn layer_tension(
    dynamic: &World,
    schema: &SchemaWorld,
    kind_map: &FxHashMap<DeclaredRelKind, InfluenceKindId>,
    threshold: Option<f32>,
    mode: SignalMode,
) -> LayerReport {
    let thresh = threshold.unwrap_or(graph_world::metrics::ACTIVITY_THRESHOLD);
    let all_kinds = distinct_dynamic_kinds(dynamic);
    let facts_by_kind = group_facts_by_kind(schema, kind_map);
    let mut layers: Vec<LayerTension> = all_kinds
        .into_iter()
        .map(|kind_id| {
            let kind_facts = facts_by_kind
                .get(&kind_id)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            build_layer_tension(dynamic, kind_id, kind_facts, thresh, mode)
        })
        .collect();

    // Sort descending by tension.
    layers.sort_by(|a, b| {
        b.tension
            .partial_cmp(&a.tension)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let most_divergent = layers.first().map(|l| l.kind);
    let most_aligned = layers.last().map(|l| l.kind);

    LayerReport {
        layers,
        most_divergent,
        most_aligned,
    }
}

fn distinct_dynamic_kinds(dynamic: &World) -> Vec<InfluenceKindId> {
    let mut all_kinds: Vec<InfluenceKindId> = dynamic
        .relationships()
        .iter()
        .map(|r| r.kind)
        .collect::<FxHashSet<_>>()
        .into_iter()
        .collect();
    all_kinds.sort_by_key(|kind| kind.0);
    all_kinds
}

fn group_facts_by_kind<'a>(
    schema: &'a SchemaWorld,
    kind_map: &FxHashMap<DeclaredRelKind, InfluenceKindId>,
) -> FxHashMap<InfluenceKindId, Vec<&'a DeclaredFact>> {
    let mut facts_by_kind: FxHashMap<InfluenceKindId, Vec<&DeclaredFact>> = FxHashMap::default();
    for fact in schema.facts.active_facts() {
        if let Some(&kind_id) = kind_map.get(&fact.predicate) {
            facts_by_kind.entry(kind_id).or_default().push(fact);
        }
    }
    facts_by_kind
}

fn build_layer_tension(
    dynamic: &World,
    kind_id: InfluenceKindId,
    kind_facts: &[&DeclaredFact],
    threshold: f32,
    mode: SignalMode,
) -> LayerTension {
    let (confirmed, ghost) = count_declared_matches(dynamic, kind_id, kind_facts, threshold, mode);
    let shadow = count_shadow_relationships(dynamic, kind_id, kind_facts, threshold, mode);
    let total = (confirmed + ghost + shadow).max(1) as f32;
    let tension = (ghost + shadow) as f32 / total;

    LayerTension {
        kind: kind_id,
        confirmed,
        ghost,
        shadow,
        tension,
    }
}

fn count_declared_matches(
    dynamic: &World,
    kind_id: InfluenceKindId,
    kind_facts: &[&DeclaredFact],
    threshold: f32,
    mode: SignalMode,
) -> (usize, usize) {
    let mut confirmed = 0usize;
    let mut ghost = 0usize;

    for &fact in kind_facts {
        let matched = dynamic
            .relationships_between(fact.subject, fact.object)
            .any(|r| r.kind == kind_id && signal(r, mode) > threshold);
        if matched {
            confirmed += 1;
        } else {
            ghost += 1;
        }
    }

    (confirmed, ghost)
}

fn count_shadow_relationships(
    dynamic: &World,
    kind_id: InfluenceKindId,
    kind_facts: &[&DeclaredFact],
    threshold: f32,
    mode: SignalMode,
) -> usize {
    let declared_pairs: FxHashSet<graph_core::EndpointKey> = kind_facts
        .iter()
        .map(|fact| graph_core::Endpoints::symmetric(fact.subject, fact.object).key())
        .collect();

    dynamic
        .relationships()
        .iter()
        .filter(|rel| {
            rel.kind == kind_id
                && signal(rel, mode) > threshold
                && !declared_pairs.contains(&rel.endpoints.key())
        })
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{
        BatchId, Endpoints, InfluenceKindId, Locus, LocusId, LocusKindId, Relationship,
        RelationshipId, RelationshipLineage, StateVector,
    };
    use graph_schema::{DeclaredRelKind, SchemaWorld};
    use graph_world::World;
    use smallvec::SmallVec;

    fn kind(s: &str) -> DeclaredRelKind {
        DeclaredRelKind::new(s)
    }

    fn make_rel(id: u64, a: u64, b: u64, kind_id: u64, strength: f32) -> Relationship {
        Relationship {
            id: RelationshipId(id),
            kind: InfluenceKindId(kind_id),
            endpoints: Endpoints::symmetric(LocusId(a), LocusId(b)),
            state: StateVector::from_slice(&[strength, 0.0]),
            lineage: RelationshipLineage {
                created_by: None,
                last_touched_by: None,
                change_count: 0,
                kinds_observed: SmallVec::new(),
            },
            created_batch: BatchId(0),
            last_decayed_batch: 0,
            metadata: None,
        }
    }

    fn make_locus(id: u64) -> Locus {
        Locus::new(LocusId(id), LocusKindId(0), StateVector::zeros(1))
    }

    #[test]
    fn fully_confirmed_layer_has_zero_tension() {
        let mut world = World::default();
        world.loci_mut().insert(make_locus(1));
        world.loci_mut().insert(make_locus(2));
        world.relationships_mut().insert(make_rel(0, 1, 2, 1, 0.9));

        let mut schema = SchemaWorld::new();
        schema.assert_fact(LocusId(1), kind("trust"), LocusId(2));

        let mut kind_map = FxHashMap::default();
        kind_map.insert(kind("trust"), InfluenceKindId(1));

        let report = layer_tension(&world, &schema, &kind_map, Some(0.1), SignalMode::Activity);
        assert_eq!(report.layers.len(), 1);
        assert_eq!(report.layers[0].tension, 0.0);
        assert_eq!(report.layers[0].confirmed, 1);
    }

    #[test]
    fn ghost_layer_has_tension_one() {
        let world = World::default(); // no dynamic rels

        let mut schema = SchemaWorld::new();
        schema.assert_fact(LocusId(1), kind("authority"), LocusId(2));

        // Authority declared but dynamic world has no rels → no layers at all
        let mut kind_map = FxHashMap::default();
        kind_map.insert(kind("authority"), InfluenceKindId(2));

        let report = layer_tension(&world, &schema, &kind_map, Some(0.1), SignalMode::Activity);
        // No layers because no dynamic relationships of kind 2 exist
        assert!(report.layers.is_empty());
    }

    #[test]
    fn two_layers_sorted_by_tension_descending() {
        // Kind 1 (trust): declared and confirmed → tension 0
        // Kind 2 (authority): declared but ghost → tension 1
        let mut world = World::default();
        world.loci_mut().insert(make_locus(1));
        world.loci_mut().insert(make_locus(2));
        world.relationships_mut().insert(make_rel(0, 1, 2, 1, 0.9)); // kind 1 confirmed

        let mut schema = SchemaWorld::new();
        schema.assert_fact(LocusId(1), kind("trust"), LocusId(2)); // confirmed
        schema.assert_fact(LocusId(1), kind("authority"), LocusId(3)); // ghost (locus 3 not connected)

        let mut kind_map = FxHashMap::default();
        kind_map.insert(kind("trust"), InfluenceKindId(1));
        kind_map.insert(kind("authority"), InfluenceKindId(1)); // both map to same kind

        let report = layer_tension(&world, &schema, &kind_map, Some(0.1), SignalMode::Activity);
        assert!(!report.layers.is_empty());
        // Layer for kind 1 should show mixed confirmed/ghost
        let l = &report.layers[0];
        assert!(l.tension > 0.0);
    }

    #[test]
    fn unmapped_predicate_does_not_appear_in_layer_confirmed() {
        let mut world = World::default();
        world.loci_mut().insert(make_locus(1));
        world.loci_mut().insert(make_locus(2));
        world.relationships_mut().insert(make_rel(0, 1, 2, 1, 0.9));

        let mut schema = SchemaWorld::new();
        schema.assert_fact(LocusId(1), kind("unmapped_pred"), LocusId(2));

        let kind_map: FxHashMap<DeclaredRelKind, InfluenceKindId> = FxHashMap::default();
        // No mapping → all active rels are shadow
        let report = layer_tension(&world, &schema, &kind_map, Some(0.1), SignalMode::Activity);
        assert_eq!(report.layers.len(), 1);
        assert_eq!(report.layers[0].shadow, 1);
        assert_eq!(report.layers[0].confirmed, 0);
        assert_eq!(report.layers[0].tension, 1.0);
    }
}
