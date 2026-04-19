//! Ring-world multi-tick dynamics example.
//!
//! Demonstrates `Simulation` running a feedback ring through its full
//! lifecycle: stimulus injection → signal cascade → relationship
//! emergence → entity recognition → quiescence.
//!
//! Also showcases newer query surface: `WorldDiff`, `WorldMetrics`,
//! `step_until`, `path_between`, and `connected_components`.
//!
//! Topology: 8 loci in a ring, each forwarding to the next (gain=0.9).
//! A stimulus is injected every 5 steps to keep Hebbian learning active.
//!
//! Run: `cargo run -p graph-engine --example ring_dynamics`

use graph_engine::{
    DefaultCoherePerspective, DefaultEmergencePerspective, DynamicsRegime, PlasticityConfig,
    Simulation,
};
use graph_query::{connected_components, path_between, reachable_from};
use graph_testkit::fixtures::{ring_world, stimulus};
use graph_testkit::programs::TEST_KIND;

fn main() {
    let (world, loci, mut influences) = ring_world(8, 0.9);

    // Enable Hebbian plasticity so relationship weights accumulate.
    if let Some(cfg) = influences.get_mut(TEST_KIND) {
        cfg.plasticity = PlasticityConfig {
            learning_rate: 0.05,
            weight_decay: 0.98,
            max_weight: 1.0,

            ..Default::default()
        };
    }

    let mut sim = Simulation::new(world, loci, influences);

    // ── Phase 1: run until the ring reaches LimitCycleSuspect ────────────────

    println!("=== Ring Dynamics (8 loci, gain=0.9) ===\n");

    let before_run = sim.world().current_batch();

    let (obs, converged) = sim.step_until(
        |o, _| matches!(o.regime, DynamicsRegime::LimitCycleSuspect),
        50,
        vec![stimulus(1.0)],
    );

    println!(
        "Reached LimitCycleSuspect after {} steps (converged={})",
        obs.len(),
        converged
    );
    println!(
        "  relationships emerged: {}",
        obs.last().unwrap().relationships
    );
    println!(
        "  guard-rail scale: {:.4}",
        obs.last()
            .unwrap()
            .scales
            .values()
            .next()
            .copied()
            .unwrap_or(1.0)
    );
    println!();

    // ── Phase 2: WorldDiff over the full run ─────────────────────────────────

    let diff = sim.world().diff_since(before_run);
    println!(
        "--- WorldDiff (batch {} → {}) ---",
        before_run.0,
        sim.world().current_batch().0
    );
    println!("  changes:               {}", diff.change_ids.len());
    println!(
        "  relationships created: {}",
        diff.relationships_created.len()
    );
    println!(
        "  relationships updated: {}",
        diff.relationships_updated.len()
    );
    println!();

    // ── Phase 3: WorldMetrics snapshot ───────────────────────────────────────

    let m = sim.world().metrics();
    println!("--- WorldMetrics ---");
    println!("  loci:              {}", m.locus_count);
    println!(
        "  relationships:     {} ({} active)",
        m.relationship_count, m.active_relationship_count
    );
    println!("  mean activity:     {:.4}", m.mean_activity);
    println!("  max activity:      {:.4}", m.max_activity);
    println!(
        "  components:        {} (largest: {} loci)",
        m.component_count, m.largest_component_size
    );
    println!("  max degree:        {}", m.max_degree);
    if let Some((lid, deg)) = m.top_loci_by_degree.first() {
        println!("  top locus by deg:  L{} ({} edges)", lid.0, deg);
    }
    println!();

    // ── Phase 4: graph traversal ──────────────────────────────────────────────

    use graph_core::LocusId;
    {
        let wg = sim.world();
        let w = &*wg;
        let path = path_between(w, LocusId(0), LocusId(4));
        if let Some(p) = path {
            let hops: Vec<String> = p.iter().map(|l| format!("L{}", l.0)).collect();
            println!("Shortest path L0 → L4: {}", hops.join(" → "));
        } else {
            println!("No path L0 → L4 (ring not yet connected)");
        }

        let reachable = reachable_from(w, LocusId(0), 3);
        println!(
            "Reachable from L0 within 3 hops: {:?}",
            reachable.iter().map(|l| l.0).collect::<Vec<_>>()
        );

        let components = connected_components(w);
        println!(
            "Connected components: {} (largest: {} loci)",
            components.len(),
            components.iter().map(Vec::len).max().unwrap_or(0)
        );
    }
    println!();

    // ── Phase 5: continue stepping every 5 steps, print table ────────────────

    println!(
        "{:<5} {:<22} {:<6} {:<8} {:<6}",
        "step", "regime", "rels", "entities", "scale"
    );
    println!("{}", "-".repeat(55));

    for step in 0..20 {
        let stimuli = if step % 5 == 0 {
            vec![stimulus(1.0)]
        } else {
            vec![]
        };
        let obs = sim.step(stimuli);
        let scale = obs.scales.values().next().copied().unwrap_or(1.0);
        println!(
            "{:<5} {:<22} {:<6} {:<8} {:.4}",
            step,
            format!("{:?}", obs.regime),
            obs.relationships,
            obs.active_entities,
            scale,
        );
    }
    println!();

    // ── Phase 6: entity recognition and cohere ────────────────────────────────

    let ep = DefaultEmergencePerspective::default();
    sim.recognize_entities(&ep);

    {
        let wg = sim.world();
        let w = &*wg;
        println!("--- Relationships emerged ---");
        for r in w.relationships().iter() {
            let (f, t) = match &r.endpoints {
                graph_core::Endpoints::Directed { from, to } => (from.0, to.0),
                _ => (0, 0),
            };
            println!(
                "  L{}→L{}  activity={:.4}  weight={:.4}  touches={}",
                f,
                t,
                r.activity(),
                r.weight(),
                r.lineage.change_count
            );
        }
        println!();

        println!("--- Entities ({} active) ---", w.entities().active_count());
        for e in w.entities().active() {
            let members: Vec<u64> = e.current.members.iter().map(|l| l.0).collect();
            println!(
                "  entity#{} members={members:?} coherence={:.3} layers={}",
                e.id.0,
                e.current.coherence,
                e.layer_count()
            );
        }
    }
    println!();

    let cp = DefaultCoherePerspective::default();
    sim.extract_cohere(&cp);
    let coheres_guard = sim.world();
    let coheres = coheres_guard.coheres().get("default").unwrap_or(&[]);
    println!("--- Coheres ({}) ---", coheres.len());
    for c in coheres {
        let ms = match &c.members {
            graph_core::CohereMembers::Entities(ids) => ids
                .iter()
                .map(|e| format!("entity#{}", e.0))
                .collect::<Vec<_>>()
                .join(", "),
            _ => "(mixed)".to_string(),
        };
        println!("  cohere#{} [{ms}] strength={:.3}", c.id.0, c.strength);
    }
    println!("\nDone.");
}
