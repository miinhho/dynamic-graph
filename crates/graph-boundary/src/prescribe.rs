//! Active inference: translate boundary tension into concrete schema proposals.
//!
//! ## Philosophy (Friston, 2010)
//!
//! In active inference, an agent minimises the gap between its **generative
//! model** (what it expects) and its **sensory data** (what it observes) by
//! either *updating its model* to fit observations, or *acting on the world*
//! to make observations fit the model.
//!
//! Here, the `SchemaWorld` is the generative model and the dynamic `World` is
//! the sensory data. `prescribe_updates` proposes model updates:
//!
//! - **Ghost → retract**: a declared fact that has been behaviourally absent
//!   for long enough is probably wrong. Propose its retraction.
//! - **Shadow → assert**: a dynamic relationship that has been behaviourally
//!   active without a declared counterpart probably deserves a declaration.
//!   Propose asserting a new fact.
//!
//! The caller decides whether to apply the proposals. The engine never mutates
//! the schema automatically.
//!
//! ## Prescription config
//!
//! ```rust,ignore
//! PrescriptionConfig {
//!     // Propose retracting a ghost fact after it has been ghost for this
//!     // many consecutive schema versions.  None = never propose retractions.
//!     ghost_version_threshold: Some(3),
//!
//!     // Propose asserting a new fact for a shadow relationship whose signal
//!     // exceeds this value.  None = never propose assertions.
//!     shadow_signal_threshold: Some(0.1),
//!
//!     // Predicate to use for auto-proposed shadow assertions.
//!     shadow_predicate: DeclaredRelKind::new("inferred_influence"),
//! }
//! ```

use graph_core::{LocusId, RelationshipId};
use graph_schema::{DeclaredFactId, DeclaredRelKind, SchemaWorld};
use graph_world::World;

use crate::analysis::{signal, SignalMode};
use crate::report::BoundaryReport;

/// Configuration for [`prescribe_updates`].
#[derive(Debug, Clone)]
pub struct PrescriptionConfig {
    /// Propose retracting a ghost fact once it has been asserted for at least
    /// this many schema versions without any dynamic confirmation.
    ///
    /// `None` disables ghost retraction proposals.
    pub ghost_version_threshold: Option<u64>,

    /// Propose asserting a new fact for shadow relationships whose signal
    /// (measured with `signal_mode`) exceeds this value.
    ///
    /// `None` disables shadow assertion proposals.
    pub shadow_signal_threshold: Option<f32>,

    /// Signal mode used when evaluating shadow relationship strength.
    pub signal_mode: SignalMode,

    /// Predicate to attach to auto-proposed shadow assertions.
    pub shadow_predicate: DeclaredRelKind,
}

impl Default for PrescriptionConfig {
    fn default() -> Self {
        PrescriptionConfig {
            ghost_version_threshold: Some(5),
            shadow_signal_threshold: Some(0.1),
            signal_mode: SignalMode::Strength,
            shadow_predicate: DeclaredRelKind::new("inferred_influence"),
        }
    }
}

/// A concrete proposal to reduce boundary tension.
#[derive(Debug, Clone, PartialEq)]
pub enum BoundaryAction {
    /// Retract a ghost fact: it has been declared but never behaviourally
    /// confirmed for `ghost_version_threshold` schema versions.
    RetractFact {
        fact_id: DeclaredFactId,
        reason: RetractReason,
    },
    /// Assert a new fact for a shadow: the dynamic relationship is behaviourally
    /// active but has no declared counterpart.
    AssertFact {
        subject: LocusId,
        predicate: DeclaredRelKind,
        object: LocusId,
        shadow_rel: RelationshipId,
    },
}

/// Why a ghost fact is being proposed for retraction.
#[derive(Debug, Clone, PartialEq)]
pub enum RetractReason {
    /// The fact has been active for `age_versions` schema versions without
    /// any behavioural confirmation.
    LongRunningGhost { age_versions: u64 },
}

/// Translate a [`BoundaryReport`] into concrete schema update proposals.
///
/// ## Retraction proposals
///
/// Every ghost edge in `report` whose corresponding fact is older than
/// `config.ghost_version_threshold` schema versions receives a
/// `BoundaryAction::RetractFact` proposal. The "age" is measured as
/// `schema.facts.version() - fact.asserted_at`, which is a rough proxy for
/// how long the fact has been sitting without confirmation.
///
/// ## Assertion proposals
///
/// Every shadow relationship in `report` whose signal (according to
/// `config.signal_mode` against `config.shadow_signal_threshold`) exceeds the
/// threshold receives a `BoundaryAction::AssertFact` proposal using
/// `config.shadow_predicate` as the predicate.
pub fn prescribe_updates(
    report: &BoundaryReport,
    schema: &SchemaWorld,
    dynamic: &World,
    config: &PrescriptionConfig,
) -> Vec<BoundaryAction> {
    let mut actions = Vec::new();

    // ── Ghost → retract proposals ─────────────────────────────────────────
    if let Some(age_threshold) = config.ghost_version_threshold {
        let current_version = schema.facts.version();

        for ghost_edge in &report.ghost {
            // Find the underlying fact.
            let maybe_fact = schema
                .facts
                .facts_between(ghost_edge.subject, &ghost_edge.predicate, ghost_edge.object)
                .next();

            if let Some(fact) = maybe_fact {
                let age = current_version.saturating_sub(fact.asserted_at);
                if age >= age_threshold {
                    actions.push(BoundaryAction::RetractFact {
                        fact_id: fact.id,
                        reason: RetractReason::LongRunningGhost { age_versions: age },
                    });
                }
            }
        }
    }

    // ── Shadow → assert proposals ─────────────────────────────────────────
    if let Some(signal_threshold) = config.shadow_signal_threshold {
        for &rel_id in &report.shadow {
            let Some(rel) = dynamic.relationships().get(rel_id) else {
                continue;
            };

            let sig = signal(rel, config.signal_mode);

            if sig < signal_threshold {
                continue;
            }

            let (subject, object) = match rel.endpoints {
                graph_core::Endpoints::Symmetric { a, b } => (a, b),
                graph_core::Endpoints::Directed { from, to } => (from, to),
            };

            actions.push(BoundaryAction::AssertFact {
                subject,
                predicate: config.shadow_predicate.clone(),
                object,
                shadow_rel: rel_id,
            });
        }
    }

    actions
}

/// Apply a list of `BoundaryAction`s directly to the `SchemaWorld`.
///
/// Returns the number of actions that resulted in an actual mutation
/// (retractions of already-retracted facts are no-ops and not counted).
pub fn apply_prescriptions(
    actions: &[BoundaryAction],
    schema: &mut SchemaWorld,
) -> usize {
    let mut applied = 0;
    for action in actions {
        match action {
            BoundaryAction::RetractFact { fact_id, .. } => {
                let before = schema.facts.version();
                schema.retract_fact(*fact_id);
                if schema.facts.version() > before {
                    applied += 1;
                }
            }
            BoundaryAction::AssertFact { subject, predicate, object, .. } => {
                let before = schema.facts.version();
                schema.assert_fact(*subject, predicate.clone(), *object);
                if schema.facts.version() > before {
                    applied += 1;
                }
            }
        }
    }
    applied
}

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{
        BatchId, Endpoints, InfluenceKindId, Locus, LocusId, LocusKindId,
        Relationship, RelationshipLineage, StateVector,
    };
    use graph_schema::{DeclaredRelKind, SchemaWorld};
    use graph_world::World;
    use smallvec::SmallVec;

    fn kind(s: &str) -> DeclaredRelKind { DeclaredRelKind::new(s) }

    fn make_world_with_rel(a: u64, b: u64, strength: f32) -> (World, RelationshipId) {
        let mut world = World::default();
        world.loci_mut().insert(Locus::new(LocusId(a), LocusKindId(0), StateVector::zeros(1)));
        world.loci_mut().insert(Locus::new(LocusId(b), LocusKindId(0), StateVector::zeros(1)));
        let rel = Relationship {
            id: RelationshipId(0),
            kind: InfluenceKindId(0),
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
        };
        let rel_id = rel.id;
        world.relationships_mut().insert(rel);
        (world, rel_id)
    }

    #[test]
    fn ghost_older_than_threshold_gets_retract_proposal() {
        let dynamic = World::default();

        let mut schema = SchemaWorld::new();
        // Bump version to 10 before asserting.
        for _ in 0..10 {
            schema.assert_fact(LocusId(99), kind("dummy"), LocusId(100));
            let id = schema.facts.active_facts().last().unwrap().id;
            schema.retract_fact(id);
        }
        let id = schema.assert_fact(LocusId(1), kind("reports_to"), LocusId(2));
        // fact asserted at version ~21, current version ~21
        // After retract, version will be higher. Let's just assert it's old enough.
        // Add more bumps to make the fact "old".
        for _ in 0..6 {
            schema.assert_fact(LocusId(98), kind("dummy2"), LocusId(97));
            let fid = schema.facts.active_facts()
                .find(|f| f.subject == LocusId(98)).unwrap().id;
            schema.retract_fact(fid);
        }
        // fact.asserted_at < current_version - 5

        let report = crate::analysis::analyze_boundary(&dynamic, &schema, None);
        // ghost because no dynamic rels

        let cfg = PrescriptionConfig {
            ghost_version_threshold: Some(5),
            shadow_signal_threshold: None,
            signal_mode: SignalMode::Activity,
            shadow_predicate: kind("inferred"),
        };
        let actions = prescribe_updates(&report, &schema, &dynamic, &cfg);

        // Should propose retraction of the reports_to fact.
        let retract_count = actions.iter().filter(|a| matches!(a, BoundaryAction::RetractFact { fact_id, .. } if *fact_id == id)).count();
        assert_eq!(retract_count, 1);
    }

    #[test]
    fn young_ghost_is_not_proposed_for_retraction() {
        let dynamic = World::default();
        let mut schema = SchemaWorld::new();
        schema.assert_fact(LocusId(1), kind("knows"), LocusId(2));
        // Asserted at version 1, current version 1 — age = 0

        let report = crate::analysis::analyze_boundary(&dynamic, &schema, None);

        let cfg = PrescriptionConfig {
            ghost_version_threshold: Some(5),
            ..Default::default()
        };
        let actions = prescribe_updates(&report, &schema, &dynamic, &cfg);
        assert!(actions.is_empty(), "too young to retract");
    }

    #[test]
    fn shadow_above_threshold_gets_assert_proposal() {
        let (dynamic, rel_id) = make_world_with_rel(5, 6, 0.9);
        let schema = SchemaWorld::new();

        let report = crate::analysis::analyze_boundary_with_mode(
            &dynamic, &schema, Some(0.1), SignalMode::Activity,
        );
        assert_eq!(report.shadow.len(), 1);

        let cfg = PrescriptionConfig {
            ghost_version_threshold: None,
            shadow_signal_threshold: Some(0.5),
            signal_mode: SignalMode::Activity,
            shadow_predicate: kind("inferred_influence"),
        };
        let actions = prescribe_updates(&report, &schema, &dynamic, &cfg);

        assert_eq!(actions.len(), 1);
        assert!(matches!(&actions[0], BoundaryAction::AssertFact {
            shadow_rel, predicate, ..
        } if *shadow_rel == rel_id && predicate == &kind("inferred_influence")));
    }

    #[test]
    fn shadow_below_threshold_is_ignored() {
        let (dynamic, _) = make_world_with_rel(5, 6, 0.05); // weak shadow
        let schema = SchemaWorld::new();

        let report = crate::analysis::analyze_boundary_with_mode(
            &dynamic, &schema, Some(0.01), SignalMode::Activity,
        );

        let cfg = PrescriptionConfig {
            ghost_version_threshold: None,
            shadow_signal_threshold: Some(0.5), // high threshold
            signal_mode: SignalMode::Activity,
            shadow_predicate: kind("inferred_influence"),
        };
        let actions = prescribe_updates(&report, &schema, &dynamic, &cfg);
        assert!(actions.is_empty());
    }

    #[test]
    fn apply_prescriptions_mutates_schema() {
        let (dynamic, _) = make_world_with_rel(1, 2, 0.9);
        let mut schema = SchemaWorld::new();

        let report = crate::analysis::analyze_boundary_with_mode(
            &dynamic, &schema, Some(0.1), SignalMode::Activity,
        );

        let cfg = PrescriptionConfig {
            ghost_version_threshold: None,
            shadow_signal_threshold: Some(0.1),
            signal_mode: SignalMode::Activity,
            shadow_predicate: kind("inferred_influence"),
        };
        let actions = prescribe_updates(&report, &schema, &dynamic, &cfg);
        assert_eq!(actions.len(), 1);

        let applied = apply_prescriptions(&actions, &mut schema);
        assert_eq!(applied, 1);
        assert_eq!(schema.facts.active_facts().count(), 1);
        assert_eq!(
            schema.facts.active_facts().next().unwrap().predicate,
            kind("inferred_influence"),
        );
    }
}
