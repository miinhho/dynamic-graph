//! G4 — boundary regression fixture.
//!
//! Reproduces the canonical scenario from
//! `examples/boundary_workflow.rs` and asserts the four-quadrant
//! counts. If the engine's auto-emergence, decay, plasticity, or
//! `graph-boundary::analyze_boundary` logic shifts, this test catches
//! the behavioural change before the example silently rots.
//!
//! The scenario:
//! - 8 people, 7 declared `reports_to` facts.
//! - 6 pairs × 6 rounds of `interact()` (predecessor-chained co-stim
//!   on each call so every round reinforces the shared relationship).
//! - Carol and Dave never interact → Ghost.
//! - Alice ↔ Eve cross-team collaboration has no declared
//!   counterpart → Shadow.
//!
//! Expected:
//! - confirmed = 5
//! - ghost     = 2
//! - shadow    = 1
//! - tension   ≈ 0.375 (tight tolerance: ±0.01)
//! - 2 RetractFact + 1 AssertFact prescriptions at default config.
//! - Post-apply tension = 0.000 (aligned).

use graph_boundary::{
    BoundaryAction, PrescriptionConfig, analyze_boundary, apply_prescriptions, locus_tension,
    prescribe_updates,
};
use graph_core::{
    Change, ChangeId, InfluenceKindId, Locus, LocusContext, LocusId, LocusProgram,
    ProposedChange, StabilizationConfig, props,
};
use graph_engine::{InfluenceKindConfig, PlasticityConfig, Simulation, SimulationBuilder};
use graph_schema::{DeclaredRelKind, SchemaWorld};

const COLLAB: InfluenceKindId = InfluenceKindId(1);

struct Noop;
impl LocusProgram for Noop {
    fn process(
        &self,
        _: &Locus,
        _: &[&Change],
        _: &dyn LocusContext,
    ) -> Vec<ProposedChange> {
        vec![]
    }
}

fn interact(sim: &mut Simulation, a: LocusId, b: LocusId) {
    let last_a: Option<ChangeId> = sim.world().log().changes_to_locus(a).next().map(|c| c.id);
    let mut stim_b = ProposedChange::activation(b, COLLAB, 1.0);
    if let Some(p) = last_a {
        stim_b = stim_b.with_extra_predecessors(vec![p]);
    }
    sim.step(vec![
        ProposedChange::activation(a, COLLAB, 1.0),
        stim_b,
    ]);
}

fn build_scenario() -> (Simulation, SchemaWorld, std::collections::HashMap<&'static str, LocusId>) {
    let cast = &[
        ("CEO",   "exec"),
        ("CTO",   "exec"),
        ("CMO",   "exec"),
        ("Alice", "eng_lead"),
        ("Bob",   "eng_lead"),
        ("Carol", "eng_ic"),
        ("Dave",  "eng_ic"),
        ("Eve",   "mkt_lead"),
    ];

    let mut sim = SimulationBuilder::new()
        .locus_kind("PERSON", Noop)
        .influence("collab", |cfg: InfluenceKindConfig| {
            cfg.with_decay(0.9)
                .with_stabilization(StabilizationConfig { alpha: 0.7 })
                .with_plasticity(PlasticityConfig {
                    learning_rate: 0.05,
                    weight_decay: 0.005,
                    max_weight: 5.0,
                    ..Default::default()
                })
        })
        .default_influence("collab")
        .build();

    let mut ids = std::collections::HashMap::new();
    for (name, role) in cast {
        let id = sim.ingest_named(*name, "PERSON", props! { "name" => *name, "role" => *role });
        ids.insert(*name, id);
    }

    let mut schema = SchemaWorld::new();
    let reports_to = DeclaredRelKind::new("reports_to");
    for (sub, obj) in &[
        ("CTO",   "CEO"),
        ("CMO",   "CEO"),
        ("Alice", "CTO"),
        ("Bob",   "CTO"),
        ("Carol", "CTO"),
        ("Dave",  "CTO"),
        ("Eve",   "CMO"),
    ] {
        schema.assert_fact(ids[sub], reports_to.clone(), ids[obj]);
    }

    // Age the facts enough that ghost retractions trigger at default
    // config (ghost_version_threshold=3).
    let filler = DeclaredRelKind::new("__filler__");
    for i in 0..12 {
        let fid = schema.assert_fact(LocusId(9_000 + i), filler.clone(), LocusId(9_500 + i));
        schema.retract_fact(fid);
    }

    let active_pairs: &[(&str, &str)] = &[
        ("CTO",   "CEO"),
        ("CMO",   "CEO"),
        ("Alice", "CTO"),
        ("Bob",   "CTO"),
        ("Eve",   "CMO"),
        ("Alice", "Eve"),
    ];
    for _ in 0..6 {
        for (a, b) in active_pairs {
            interact(&mut sim, ids[a], ids[b]);
        }
    }

    (sim, schema, ids)
}

#[test]
fn boundary_workflow_fixture_produces_expected_quadrants() {
    let (sim, schema, _) = build_scenario();
    let report = analyze_boundary(&*sim.world(), &schema, Some(0.05));

    assert_eq!(report.confirmed.len(), 5, "confirmed edges");
    assert_eq!(report.ghost.len(), 2, "ghost edges");
    assert_eq!(report.shadow.len(), 1, "shadow edges");

    let expected_tension = (2.0 + 1.0) / (5.0 + 2.0 + 1.0);
    assert!(
        (report.tension - expected_tension).abs() < 0.01,
        "tension {:.4} far from expected {:.4}",
        report.tension,
        expected_tension,
    );
    assert!(!report.is_aligned());
}

#[test]
fn prescribe_updates_produces_two_retractions_and_one_assertion() {
    let (sim, schema, ids) = build_scenario();
    let report = analyze_boundary(&*sim.world(), &schema, Some(0.05));

    let config = PrescriptionConfig {
        ghost_version_threshold: Some(3),
        shadow_signal_threshold: Some(0.1),
        shadow_predicate: DeclaredRelKind::new("inferred_collab"),
        ..PrescriptionConfig::default()
    };
    let actions = prescribe_updates(&report, &schema, &*sim.world(), &config);
    assert_eq!(actions.len(), 3);

    let retract_count = actions
        .iter()
        .filter(|a| matches!(a, BoundaryAction::RetractFact { .. }))
        .count();
    let assert_count = actions
        .iter()
        .filter(|a| matches!(a, BoundaryAction::AssertFact { .. }))
        .count();
    assert_eq!(retract_count, 2, "expected 2 retractions (Carol, Dave)");
    assert_eq!(assert_count, 1, "expected 1 assertion (Alice↔Eve)");

    let alice = ids["Alice"];
    let eve = ids["Eve"];
    let asserted = actions
        .iter()
        .find_map(|a| match a {
            BoundaryAction::AssertFact { subject, object, .. } => Some((*subject, *object)),
            _ => None,
        })
        .expect("assertion action present");
    assert!(
        (asserted.0 == alice && asserted.1 == eve) || (asserted.0 == eve && asserted.1 == alice),
        "shadow assertion should be between Alice and Eve, got ({:?}, {:?})",
        asserted.0,
        asserted.1,
    );
}

#[test]
fn cto_is_the_per_locus_hotspot() {
    let (sim, schema, ids) = build_scenario();
    let report = analyze_boundary(&*sim.world(), &schema, Some(0.05));
    let rows = locus_tension(&report, &*sim.world());

    // CTO receives both Carol→CTO and Dave→CTO ghosts, so it carries
    // the highest absolute drift count in the scenario.
    let top = rows.first().expect("at least one locus ranked");
    assert_eq!(top.locus, ids["CTO"], "CTO expected to top the ranking");
    assert_eq!(top.ghost, 2, "CTO ghost count");
    assert_eq!(top.shadow, 0, "CTO shadow count");

    // Carol and Dave should each show a single ghost with no confirmed.
    for name in &["Carol", "Dave"] {
        let row = rows
            .iter()
            .find(|r| r.locus == ids[*name])
            .unwrap_or_else(|| panic!("{name} missing from per-locus breakdown"));
        assert_eq!(row.confirmed, 0);
        assert_eq!(row.ghost, 1);
        assert_eq!(row.shadow, 0);
        assert_eq!(row.tension, 1.0);
    }

    // Alice and Eve: 1 confirmed + 1 shadow each (cross-team collab).
    for name in &["Alice", "Eve"] {
        let row = rows
            .iter()
            .find(|r| r.locus == ids[*name])
            .unwrap_or_else(|| panic!("{name} missing"));
        assert_eq!(row.confirmed, 1);
        assert_eq!(row.shadow, 1);
    }
}

#[test]
fn applying_prescriptions_drops_tension_to_zero() {
    let (sim, mut schema, _) = build_scenario();
    let report = analyze_boundary(&*sim.world(), &schema, Some(0.05));

    let config = PrescriptionConfig {
        ghost_version_threshold: Some(3),
        shadow_signal_threshold: Some(0.1),
        shadow_predicate: DeclaredRelKind::new("inferred_collab"),
        ..PrescriptionConfig::default()
    };
    let actions = prescribe_updates(&report, &schema, &*sim.world(), &config);
    let applied = apply_prescriptions(&actions, &mut schema);
    assert_eq!(applied, actions.len(), "all prescribed actions should apply");

    let after = analyze_boundary(&*sim.world(), &schema, Some(0.05));
    assert_eq!(after.ghost.len(), 0, "post-apply ghosts");
    assert_eq!(after.shadow.len(), 0, "post-apply shadows");
    assert!(
        after.is_aligned(),
        "post-apply boundary should be aligned (tension={})",
        after.tension,
    );
}
