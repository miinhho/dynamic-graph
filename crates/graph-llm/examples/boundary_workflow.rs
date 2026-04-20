//! End-to-end declared-vs-observed drift workflow.
//!
//! Walks the full `graph-boundary` pipeline against a tiny organisational
//! scenario so every public stage is exercised in one file:
//!
//! 1. Declare a small organisational hierarchy in a `SchemaWorld` (who
//!    reports to whom, who collaborates with whom).
//! 2. Run a dynamic simulation where observed interaction drifts from the
//!    declared hierarchy:
//!    - the CTO and the eng leads really do collaborate (→ Confirmed),
//!    - two declared direct reports never talk to anybody (→ Ghost),
//!    - an eng lead and a marketing lead co-drive a cross-team project
//!      that no-one ever formalised (→ Shadow),
//!    - most pairs neither declared nor active (→ Null, not reported).
//! 3. `analyze_boundary(&dynamic, &schema, threshold)` → `BoundaryReport`
//!    listing Confirmed / Ghost / Shadow edges and the overall tension
//!    score in [0, 1].
//! 4. `prescribe_updates(..)` → `Vec<BoundaryAction>` (retract stale
//!    ghosts, assert newly-observed shadows).
//! 5. `narrate_prescriptions(..)` with `MockLlmClient` so the example is
//!    hermetic (no network, no API key). Substitute `AnthropicClient` or
//!    `OllamaClient` for real narration.
//! 6. `apply_prescriptions(..)` mutates the schema in-place so a follow-up
//!    `analyze_boundary` shows reduced tension.
//!
//! Run: `cargo run -p graph-llm --example boundary_workflow`

use graph_boundary::{
    BoundaryAction, BoundaryReport, PrescriptionConfig, RetractReason, analyze_boundary,
    apply_prescriptions, prescribe_updates,
};
use graph_core::{
    Change, ChangeId, InfluenceKindId, Locus, LocusContext, LocusId, LocusProgram,
    ProposedChange, StabilizationConfig, props,
};
use graph_engine::{InfluenceKindConfig, PlasticityConfig, Simulation, SimulationBuilder};
use graph_llm::{MockLlmClient, narrate_prescriptions};
use graph_query::NameMap;
use graph_schema::{DeclaredRelKind, SchemaWorld};

const COLLAB: InfluenceKindId = InfluenceKindId(1);

// ── LocusProgram ──────────────────────────────────────────────────────────────

/// Noop program. This example wants clean, predictable relationship
/// creation driven by `interact()` alone; re-emission would generate
/// secondary relationships that pollute the shadow set.
struct PersonProgram;

impl LocusProgram for PersonProgram {
    fn process(
        &self,
        _locus: &Locus,
        _incoming: &[&Change],
        _ctx: &dyn LocusContext,
    ) -> Vec<ProposedChange> {
        vec![]
    }
}

// ── Interaction helper ────────────────────────────────────────────────────────

/// Step the sim with a co-activation of `(a, b)`. Uses predecessor-
/// chaining on every call so each co-stim delivers activity to the
/// shared relationship (without the chain the engine has no reason to
/// emit a Change against the rel and the activity simply decays).
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

// ── Main ──────────────────────────────────────────────────────────────────────

fn main() {
    // Cast of characters. The IDs are assigned by `ingest_named` at first use.
    let cast = &[
        ("CEO",   "exec"),
        ("CTO",   "exec"),
        ("CMO",   "exec"),
        ("Alice", "eng_lead"),
        ("Bob",   "eng_lead"),
        ("Carol", "eng_ic"),   // declared but will never interact → ghost target
        ("Dave",  "eng_ic"),   // ditto
        ("Eve",   "mkt_lead"),
    ];

    let mut sim = SimulationBuilder::new()
        .locus_kind("PERSON", PersonProgram)
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

    // Seed every cast member into the dynamic world so that schema facts
    // resolve to existing loci (Ghost edges need the locus to exist on
    // both sides).
    let mut ids: std::collections::HashMap<&'static str, LocusId> =
        std::collections::HashMap::new();
    for (name, role) in cast {
        let id = sim.ingest_named(*name, "PERSON", props! { "name" => *name, "role" => *role });
        ids.insert(name, id);
    }

    // ── 1. Declare the org chart (SchemaWorld) ─────────────────────────────
    let mut schema = SchemaWorld::new();
    let reports_to = DeclaredRelKind::new("reports_to");
    let declared: Vec<(&'static str, &'static str)> = vec![
        ("CTO",   "CEO"),
        ("CMO",   "CEO"),
        ("Alice", "CTO"),
        ("Bob",   "CTO"),
        ("Carol", "CTO"),   // will be ghost: Carol never interacts
        ("Dave",  "CTO"),   // will be ghost: Dave never interacts
        ("Eve",   "CMO"),
    ];
    let mut declared_fact_ids = Vec::new();
    for (sub, obj) in &declared {
        let fact_id = schema.assert_fact(ids[sub], reports_to.clone(), ids[obj]);
        declared_fact_ids.push((*sub, *obj, fact_id));
    }

    // Bump schema version so the Ghost facts are "old enough" when we
    // prescribe retractions (age = current_version - asserted_at).
    let filler_kind = DeclaredRelKind::new("__version_filler__");
    for i in 0..12 {
        let a = LocusId(9_000 + i);
        let b = LocusId(9_500 + i);
        let fid = schema.assert_fact(a, filler_kind.clone(), b);
        schema.retract_fact(fid);
    }

    // ── 2. Run the dynamic simulation ──────────────────────────────────────
    // Interleave the active pairs across rounds so no single pair's
    // relationship decays below threshold while a later pair is still
    // being stimulated. The Alice-Eve pair has no declared counterpart
    // and becomes the Shadow edge; Carol and Dave receive no
    // interactions and their declared reports_to stays Ghost.
    let active_pairs: &[(&str, &str)] = &[
        ("CTO",   "CEO"),
        ("CMO",   "CEO"),
        ("Alice", "CTO"),
        ("Bob",   "CTO"),
        ("Eve",   "CMO"),
        ("Alice", "Eve"), // SHADOW — undeclared cross-team collaboration
    ];
    let rounds = 6;
    for _ in 0..rounds {
        for (a, b) in active_pairs {
            interact(&mut sim, ids[a], ids[b]);
        }
    }

    println!("\n── Dynamic world after simulation ──");
    println!("  relationships = {}", sim.world().relationships().len());
    println!("  loci          = {}", sim.world().loci().iter().count());
    {
        let w = sim.world();
        let names = NameMap::from_world(&*w);
        for rel in w.relationships().iter() {
            let (a, b) = match rel.endpoints {
                graph_core::Endpoints::Directed { from, to } => (from, to),
                graph_core::Endpoints::Symmetric { a, b } => (a, b),
            };
            println!(
                "    {} ↔ {}  activity={:.3} weight={:.3} strength={:.3}",
                names.name(a),
                names.name(b),
                rel.activity(),
                rel.weight(),
                rel.strength(),
            );
        }
    }

    // ── 3. Boundary analysis ───────────────────────────────────────────────
    let report = analyze_boundary(&*sim.world(), &schema, Some(0.05));
    print_report("Initial boundary report", &report, &*sim.world(), &schema);

    // ── 4. Prescriptions ───────────────────────────────────────────────────
    let config = PrescriptionConfig {
        ghost_version_threshold: Some(3),
        shadow_signal_threshold: Some(0.1),
        shadow_predicate: DeclaredRelKind::new("inferred_collab"),
        ..PrescriptionConfig::default()
    };
    let actions = prescribe_updates(&report, &schema, &*sim.world(), &config);
    println!("\n── Prescribed actions ({}) ──", actions.len());
    for action in &actions {
        print_action(action, &schema, &*sim.world(), &ids);
    }

    // ── 5. Narrate via MockLlmClient ───────────────────────────────────────
    // Swap `MockLlmClient::new(...)` for `AnthropicClient::from_env()` or
    // `OllamaClient::new(...)` when a real model should respond.
    let names = NameMap::from_world(&*sim.world());
    let client = MockLlmClient::new(
        "Two declared reporting lines (Carol→CTO and Dave→CTO) have not been exercised \
         in observed behaviour — treat them as stale structure unless you can name a \
         plausible reason they stayed latent. Meanwhile Alice and Eve are collaborating \
         across the eng/marketing boundary without any formal declaration — consider \
         making that working relationship explicit."
    );
    match narrate_prescriptions(&client, &actions, &schema, &names) {
        Ok(prose) => println!("\n── Narration ──\n{prose}"),
        Err(e) => eprintln!("narrate_prescriptions failed: {e}"),
    }

    // ── 6. Apply and re-analyse ────────────────────────────────────────────
    let applied = apply_prescriptions(&actions, &mut schema);
    println!("\napplied {applied} of {} action(s)", actions.len());
    let report_after = analyze_boundary(&*sim.world(), &schema, Some(0.05));
    print_report("Post-apply boundary report", &report_after, &*sim.world(), &schema);

    println!(
        "\ntension: {:.3} → {:.3}  (Δ = {:+.3})",
        report.tension,
        report_after.tension,
        report_after.tension - report.tension,
    );
}

// ── Pretty printers ───────────────────────────────────────────────────────────

fn name_of(names: &NameMap, id: LocusId) -> String {
    names.name(id).to_string()
}

fn print_report(label: &str, r: &BoundaryReport, world: &graph_world::World, _schema: &SchemaWorld) {
    let names = NameMap::from_world(world);
    println!("\n── {label} ──");
    println!(
        "  confirmed={}  ghost={}  shadow={}  tension={:.3}",
        r.confirmed.len(),
        r.ghost.len(),
        r.shadow.len(),
        r.tension
    );
    if !r.confirmed.is_empty() {
        println!("  CONFIRMED:");
        for edge in &r.confirmed {
            println!(
                "    {} -[{}]→ {}",
                name_of(&names, edge.subject),
                edge.predicate,
                name_of(&names, edge.object),
            );
        }
    }
    if !r.ghost.is_empty() {
        println!("  GHOST:");
        for edge in &r.ghost {
            println!(
                "    {} -[{}]→ {}",
                name_of(&names, edge.subject),
                edge.predicate,
                name_of(&names, edge.object),
            );
        }
    }
    if !r.shadow.is_empty() {
        println!("  SHADOW:");
        for rel_id in &r.shadow {
            if let Some(rel) = world.relationships().get(*rel_id) {
                let (a, b) = match rel.endpoints {
                    graph_core::Endpoints::Directed { from, to } => (from, to),
                    graph_core::Endpoints::Symmetric { a, b } => (a, b),
                };
                println!(
                    "    {} ↔ {}  (strength={:.3})",
                    name_of(&names, a),
                    name_of(&names, b),
                    rel.strength(),
                );
            }
        }
    }
}

fn print_action(
    action: &BoundaryAction,
    schema: &SchemaWorld,
    _world: &graph_world::World,
    ids: &std::collections::HashMap<&'static str, LocusId>,
) {
    let lookup = |id: LocusId| -> String {
        ids.iter()
            .find(|(_, v)| **v == id)
            .map(|(k, _)| k.to_string())
            .unwrap_or_else(|| format!("locus#{}", id.0))
    };
    match action {
        BoundaryAction::RetractFact { fact_id, reason } => {
            let desc = schema
                .facts
                .active_facts()
                .find(|f| f.id == *fact_id)
                .map(|f| format!("{} -[{}]→ {}", lookup(f.subject), f.predicate, lookup(f.object)))
                .unwrap_or_else(|| format!("fact#{}", fact_id.0));
            let age = match reason {
                RetractReason::LongRunningGhost { age_versions } => *age_versions,
            };
            println!("  RETRACT  {desc}  (ghost for {age} versions)");
        }
        BoundaryAction::AssertFact {
            subject,
            predicate,
            object,
            ..
        } => {
            println!(
                "  ASSERT   {} -[{}]→ {}",
                lookup(*subject),
                predicate,
                lookup(*object),
            );
        }
    }
}
