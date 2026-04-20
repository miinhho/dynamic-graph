//! Per-locus drift breakdown. `layer_tension` aggregates per kind; this
//! module aggregates per *node*, so callers can answer "which loci are
//! the hotspots of declared-vs-observed tension?"
//!
//! Every [`crate::BoundaryEdge`] in a [`crate::BoundaryReport`] touches
//! two loci; every shadow `RelationshipId` resolves to two endpoints.
//! [`locus_tension`] sums those touches into a per-locus counter and
//! returns the loci sorted descending by local tension.

use rustc_hash::FxHashMap;

use graph_core::{Endpoints, LocusId};
use graph_world::World;

use crate::report::BoundaryReport;

/// Drift summary for a single locus.
#[derive(Debug, Clone, PartialEq)]
pub struct LocusTension {
    pub locus: LocusId,
    /// Number of confirmed edges incident to this locus.
    pub confirmed: usize,
    /// Number of ghost edges incident to this locus.
    pub ghost: usize,
    /// Number of shadow relationships incident to this locus.
    pub shadow: usize,
    /// `(ghost + shadow) / (confirmed + ghost + shadow).max(1)`.
    /// 0.0 = all incident edges are behaviourally confirmed;
    /// 1.0 = none of this locus's declared/active structure matches.
    pub tension: f32,
}

impl LocusTension {
    pub fn total(&self) -> usize {
        self.confirmed + self.ghost + self.shadow
    }
}

/// Aggregate a [`BoundaryReport`] into per-locus drift counts and
/// return the result sorted descending by `tension`. Loci with zero
/// incidence are omitted.
///
/// The `world` reference is used to resolve shadow `RelationshipId`s
/// back to their endpoint loci.
pub fn locus_tension(report: &BoundaryReport, world: &World) -> Vec<LocusTension> {
    let mut counts: FxHashMap<LocusId, [usize; 3]> = FxHashMap::default();
    // [confirmed, ghost, shadow]
    for edge in &report.confirmed {
        counts.entry(edge.subject).or_insert([0, 0, 0])[0] += 1;
        counts.entry(edge.object).or_insert([0, 0, 0])[0] += 1;
    }
    for edge in &report.ghost {
        counts.entry(edge.subject).or_insert([0, 0, 0])[1] += 1;
        counts.entry(edge.object).or_insert([0, 0, 0])[1] += 1;
    }
    for &rel_id in &report.shadow {
        if let Some(rel) = world.relationships().get(rel_id) {
            let (a, b) = match rel.endpoints {
                Endpoints::Directed { from, to } => (from, to),
                Endpoints::Symmetric { a, b } => (a, b),
            };
            counts.entry(a).or_insert([0, 0, 0])[2] += 1;
            counts.entry(b).or_insert([0, 0, 0])[2] += 1;
        }
    }

    let mut out: Vec<LocusTension> = counts
        .into_iter()
        .map(|(locus, [confirmed, ghost, shadow])| {
            let total = (confirmed + ghost + shadow).max(1);
            LocusTension {
                locus,
                confirmed,
                ghost,
                shadow,
                tension: (ghost + shadow) as f32 / total as f32,
            }
        })
        .collect();

    // Sort by absolute drift count (ghost + shadow) descending. This
    // makes the result a ranked "hotspot" list — a locus with 5 ghost
    // edges out of 10 incidences ranks above a locus with 1 ghost edge
    // out of 1, because the first is where more investigation-worthy
    // drift is concentrated even though its ratio is lower. Ties break
    // by tension ratio, then by locus id for determinism.
    out.sort_by(|a, b| {
        (b.ghost + b.shadow)
            .cmp(&(a.ghost + a.shadow))
            .then_with(|| {
                b.tension
                    .partial_cmp(&a.tension)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.locus.0.cmp(&b.locus.0))
    });
    out
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

    fn kind(s: &str) -> DeclaredRelKind {
        DeclaredRelKind::new(s)
    }

    #[test]
    fn aligned_world_produces_zero_tension_per_locus() {
        let mut world = World::default();
        world.loci_mut().insert(make_locus(1));
        world.loci_mut().insert(make_locus(2));
        world.relationships_mut().insert(make_rel(0, 1, 2, 0.9));

        let mut schema = SchemaWorld::new();
        schema.assert_fact(LocusId(1), kind("knows"), LocusId(2));

        let report = crate::analysis::analyze_boundary(&world, &schema, Some(0.1));
        let out = locus_tension(&report, &world);

        // Both loci appear with confirmed=1 and tension 0.0.
        assert_eq!(out.len(), 2);
        for row in &out {
            assert_eq!(row.confirmed, 1);
            assert_eq!(row.ghost, 0);
            assert_eq!(row.shadow, 0);
            assert_eq!(row.tension, 0.0);
        }
    }

    #[test]
    fn hotspot_locus_ranks_highest() {
        // Alice is the hotspot: she has 1 confirmed + 2 ghosts + 1 shadow.
        // Bob has 1 confirmed only (clean).
        // Carol has 1 ghost only (fully ghost).
        let mut world = World::default();
        for id in [1u64, 2, 3, 4] {
            world.loci_mut().insert(make_locus(id));
        }
        world.relationships_mut().insert(make_rel(0, 1, 2, 0.9)); // Alice–Bob confirmed
        world.relationships_mut().insert(make_rel(1, 1, 4, 0.9)); // Alice–Dave shadow

        let mut schema = SchemaWorld::new();
        schema.assert_fact(LocusId(1), kind("x"), LocusId(2)); // Alice-Bob confirmed
        schema.assert_fact(LocusId(1), kind("y"), LocusId(99)); // Alice ghost 1 (locus 99 absent → still counted)
        schema.assert_fact(LocusId(1), kind("z"), LocusId(3)); // Alice-Carol ghost

        let report = crate::analysis::analyze_boundary(&world, &schema, Some(0.1));
        let out = locus_tension(&report, &world);

        let alice = out
            .iter()
            .find(|l| l.locus == LocusId(1))
            .expect("Alice present");
        assert_eq!(alice.confirmed, 1);
        assert_eq!(alice.ghost, 2);
        assert_eq!(alice.shadow, 1);
        // (2 + 1) / (1 + 2 + 1) = 0.75
        assert!((alice.tension - 0.75).abs() < 1e-6);

        // Alice should be the top-ranked locus in the result.
        assert_eq!(out[0].locus, LocusId(1));
    }

    #[test]
    fn shadow_without_resolvable_rel_is_skipped() {
        let world = World::default(); // no rels registered
        let report = BoundaryReport {
            confirmed: vec![],
            ghost: vec![],
            shadow: vec![RelationshipId(999)], // dangling reference
            tension: 1.0,
        };
        let out = locus_tension(&report, &world);
        assert!(
            out.is_empty(),
            "dangling shadow ids should not materialise loci"
        );
    }

    #[test]
    fn total_and_tension_are_consistent() {
        let mut world = World::default();
        world.loci_mut().insert(make_locus(1));
        world.loci_mut().insert(make_locus(2));
        world.relationships_mut().insert(make_rel(0, 1, 2, 0.9));

        let schema = SchemaWorld::new();
        let report = crate::analysis::analyze_boundary(&world, &schema, Some(0.1));
        let out = locus_tension(&report, &world);
        for row in &out {
            assert_eq!(row.total(), row.confirmed + row.ghost + row.shadow);
            let denom = row.total().max(1) as f32;
            let expected = (row.ghost + row.shadow) as f32 / denom;
            assert!((row.tension - expected).abs() < 1e-6);
        }
    }
}
