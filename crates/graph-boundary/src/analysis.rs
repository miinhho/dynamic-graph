//! Core boundary analysis logic.

use rustc_hash::FxHashSet;

use graph_core::{Relationship, RelationshipId};
use graph_schema::SchemaWorld;
use graph_world::World;

use crate::report::{BoundaryEdge, BoundaryReport};
use graph_world::metrics::ACTIVITY_THRESHOLD;

/// The signal used to determine whether a dynamic relationship is "alive".
///
/// After long simulations, `Activity` can decay to near-zero even for
/// structurally important edges. `Weight` (Hebbian weight, no decay) and
/// `Strength` (activity + weight) provide more durable signals when the
/// caller wants to measure accumulated behavioural reinforcement rather
/// than current instantaneous activity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SignalMode {
    /// Use `rel.activity()` — instantaneous signal.  Decays between batches.
    Activity,
    /// Use `rel.weight()` — accumulated Hebbian weight.  Zero if plasticity
    /// was disabled; grows when pre/post loci are co-activated.
    Weight,
    /// Use `rel.strength()` = `activity + weight`.  Best of both: captures
    /// current signal AND learned importance.
    Strength,
}

impl Default for SignalMode {
    fn default() -> Self {
        Self::Strength
    }
}

pub(crate) fn signal(rel: &Relationship, mode: SignalMode) -> f32 {
    match mode {
        SignalMode::Activity => rel.activity(),
        SignalMode::Weight => rel.weight(),
        SignalMode::Strength => rel.strength(),
    }
}

/// Analyse the boundary between a dynamic [`World`] and a static [`SchemaWorld`].
///
/// ## Matching logic
///
/// A declared fact `(subject, predicate, object)` is **confirmed** if the
/// dynamic world contains at least one relationship between those two loci
/// (in either direction) whose signal exceeds `threshold`.
///
/// It is **ghost** otherwise: declared but behaviourally absent.
///
/// Any dynamic relationship with signal above the threshold that is NOT
/// covered by any declared fact (in either direction) becomes a **shadow**.
///
/// ## Parameters
///
/// - `threshold`: `None` defaults to [`ACTIVITY_THRESHOLD`] from `graph-world`.
/// - `mode`: which signal to threshold on.  Defaults to [`SignalMode::Strength`]
///   (activity + Hebbian weight), which degrades gracefully under heavy decay.
pub fn analyze_boundary(
    dynamic: &World,
    schema: &SchemaWorld,
    threshold: Option<f32>,
) -> BoundaryReport {
    analyze_boundary_with_mode(dynamic, schema, threshold, SignalMode::default())
}

/// Like [`analyze_boundary`] but lets the caller choose the [`SignalMode`].
pub fn analyze_boundary_with_mode(
    dynamic: &World,
    schema: &SchemaWorld,
    threshold: Option<f32>,
    mode: SignalMode,
) -> BoundaryReport {
    let threshold = threshold.unwrap_or(ACTIVITY_THRESHOLD);

    // Collect all active dynamic relationship IDs into a set so we can
    // subtract the ones covered by declared facts to find shadows.
    let active_dynamic: FxHashSet<RelationshipId> = dynamic
        .relationships()
        .iter()
        .filter(|r| signal(r, mode) > threshold)
        .map(|r| r.id)
        .collect();

    let mut confirmed = Vec::new();
    let mut ghost = Vec::new();
    // Track which dynamic rels are "explained" by at least one declared fact.
    let mut covered: FxHashSet<RelationshipId> = FxHashSet::default();

    for fact in schema.facts.active_facts() {
        // Look for an active dynamic relationship between subject and object
        // in either direction.
        let matching_rel = dynamic
            .relationships_between(fact.subject, fact.object)
            .find(|r| active_dynamic.contains(&r.id));

        let edge = BoundaryEdge {
            subject: fact.subject,
            predicate: fact.predicate.clone(),
            object: fact.object,
            dynamic_rel: matching_rel.map(|r| r.id),
        };

        if let Some(rel) = matching_rel {
            covered.insert(rel.id);
            confirmed.push(edge);
        } else {
            ghost.push(edge);
        }
    }

    // Shadows: active dynamic rels with no declared counterpart.
    let shadow: Vec<RelationshipId> = active_dynamic
        .iter()
        .filter(|id| !covered.contains(id))
        .copied()
        .collect();

    let total = (confirmed.len() + ghost.len() + shadow.len()).max(1) as f32;
    let divergence = (ghost.len() + shadow.len()) as f32;
    let tension = divergence / total;

    BoundaryReport {
        confirmed,
        ghost,
        shadow,
        tension,
    }
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

    fn make_locus(id: u64) -> Locus {
        Locus::new(LocusId(id), LocusKindId(0), StateVector::zeros(1))
    }

    fn make_rel(id: u64, a: u64, b: u64, activity: f32) -> Relationship {
        Relationship {
            id: RelationshipId(id),
            kind: InfluenceKindId(0),
            endpoints: Endpoints::symmetric(LocusId(a), LocusId(b)),
            state: StateVector::from_slice(&[activity, 0.0]),
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

    // Build a minimal dynamic world with a relationship between two loci.
    fn world_with_active_rel(a: u64, b: u64, activity: f32) -> (World, RelationshipId) {
        let mut world = World::default();
        world.loci_mut().insert(make_locus(a));
        world.loci_mut().insert(make_locus(b));
        let rel = make_rel(0, a, b, activity);
        let rel_id = rel.id;
        world.relationships_mut().insert(rel);
        (world, rel_id)
    }

    fn kind(s: &str) -> DeclaredRelKind {
        DeclaredRelKind::new(s)
    }

    #[test]
    fn confirmed_when_declared_and_active() {
        let (dynamic, rel_id) = world_with_active_rel(1, 2, 0.9);
        let mut schema = SchemaWorld::new();
        schema.assert_fact(LocusId(1), kind("knows"), LocusId(2));

        let report = analyze_boundary(&dynamic, &schema, Some(0.1));
        assert_eq!(report.confirmed.len(), 1);
        assert_eq!(report.ghost.len(), 0);
        assert_eq!(report.shadow.len(), 0);
        assert_eq!(report.confirmed[0].dynamic_rel, Some(rel_id));
        assert_eq!(report.tension, 0.0);
    }

    #[test]
    fn ghost_when_declared_but_dynamic_absent() {
        let dynamic = World::default();
        let mut schema = SchemaWorld::new();
        schema.assert_fact(LocusId(1), kind("manages"), LocusId(3));

        let report = analyze_boundary(&dynamic, &schema, Some(0.1));
        assert_eq!(report.ghost.len(), 1);
        assert_eq!(report.confirmed.len(), 0);
        assert_eq!(report.shadow.len(), 0);
        assert_eq!(report.tension, 1.0);
    }

    #[test]
    fn shadow_when_active_but_undeclared() {
        let (dynamic, rel_id) = world_with_active_rel(5, 6, 0.8);
        let schema = SchemaWorld::new(); // no facts declared

        let report = analyze_boundary(&dynamic, &schema, Some(0.1));
        assert_eq!(report.shadow.len(), 1);
        assert_eq!(report.shadow[0], rel_id);
        assert_eq!(report.confirmed.len(), 0);
        assert_eq!(report.ghost.len(), 0);
        assert_eq!(report.tension, 1.0);
    }

    #[test]
    fn mixed_all_three_quadrants() {
        // confirmed: loci (1,2) — both declared and active
        // ghost:     loci (3,4) — declared but no dynamic rel
        // shadow:    loci (5,6) — active but not declared
        let mut world = World::default();
        world.loci_mut().insert(make_locus(1));
        world.loci_mut().insert(make_locus(2));
        world.loci_mut().insert(make_locus(5));
        world.loci_mut().insert(make_locus(6));
        world.relationships_mut().insert(make_rel(0, 1, 2, 0.9));
        let r56_id = RelationshipId(1);
        world.relationships_mut().insert(make_rel(1, 5, 6, 0.9));

        let mut schema = SchemaWorld::new();
        schema.assert_fact(LocusId(1), kind("collab"), LocusId(2)); // confirmed
        schema.assert_fact(LocusId(3), kind("reports_to"), LocusId(4)); // ghost

        let report = analyze_boundary(&world, &schema, Some(0.1));
        assert_eq!(report.confirmed.len(), 1, "confirmed");
        assert_eq!(report.ghost.len(), 1, "ghost");
        assert_eq!(report.shadow.len(), 1, "shadow");
        assert_eq!(report.shadow[0], r56_id);
        // tension = (1 ghost + 1 shadow) / 3 total
        let expected = 2.0 / 3.0;
        assert!((report.tension - expected).abs() < 1e-6);
    }

    #[test]
    fn dormant_rel_below_threshold_counts_as_ghost() {
        let (dynamic, _) = world_with_active_rel(1, 2, 0.05); // below threshold
        let mut schema = SchemaWorld::new();
        schema.assert_fact(LocusId(1), kind("knows"), LocusId(2));

        let report = analyze_boundary(&dynamic, &schema, Some(0.1));
        assert_eq!(report.ghost.len(), 1);
        assert_eq!(report.confirmed.len(), 0);
    }
}
