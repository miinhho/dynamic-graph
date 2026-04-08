//! Ring-world multi-tick dynamics example.
//!
//! Demonstrates `Simulation` running a feedback ring through its full
//! lifecycle: stimulus injection → signal cascade → relationship
//! emergence → entity recognition → quiescence.
//!
//! Topology: 8 loci in a ring, each forwarding to the next (gain=0.9).
//! A single stimulus is injected at step 0; subsequent steps run with
//! no external input and let the signal attenuate naturally.
//!
//! After each step the example prints the dynamical regime, relationship
//! count, active entity count, and the guard-rail scale for the signal
//! influence kind.
//!
//! Run: `cargo run -p graph-engine --example ring_dynamics`

use graph_engine::{
    DefaultCoherePerspective, DefaultEmergencePerspective, PlasticityConfig, Simulation,
};
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
        };
    }

    let mut sim = Simulation::new(world, loci, influences);

    println!("=== Ring Dynamics (8 loci, gain=0.9) ===\n");
    println!("{:<5} {:<22} {:<6} {:<8} {:<6}", "step", "regime", "rels", "entities", "scale");
    println!("{}", "-".repeat(55));

    // Re-stimulate every 5 steps to keep Hebbian learning active.
    for step in 0..25 {
        let stimuli = if step % 5 == 0 { vec![stimulus(1.0)] } else { vec![] };
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

    // Recognize entities once the ring has settled.
    let ep = DefaultEmergencePerspective {
        min_activity_threshold: 0.05,
        ..Default::default()
    };
    sim.recognize_entities(&ep);

    println!("--- Relationships emerged ---");
    for r in sim.world.relationships().iter() {
        let (f, t) = match &r.endpoints {
            graph_core::Endpoints::Directed { from, to } => (from.0, to.0),
            _ => (0, 0),
        };
        println!("  L{}→L{}  activity={:.4}  weight={:.4}  touches={}",
            f, t, r.activity(), r.weight(), r.lineage.change_count);
    }
    println!();

    println!("--- Entities ({} active) ---", sim.world.entities().active_count());
    for e in sim.world.entities().active() {
        let members: Vec<u64> = e.current.members.iter().map(|l| l.0).collect();
        println!("  entity#{} members={members:?} coherence={:.3} layers={}",
            e.id.0, e.current.coherence, e.layer_count());
    }
    println!();

    // Cohere extraction.
    let cp = DefaultCoherePerspective {
        min_bridge_activity: 0.01,
        ..Default::default()
    };
    sim.extract_cohere(&cp);
    let coheres = sim.world.coheres().get("default").unwrap_or(&[]);
    println!("--- Coheres ({}) ---", coheres.len());
    for c in coheres {
        let ms = match &c.members {
            graph_core::CohereMembers::Entities(ids) =>
                ids.iter().map(|e| format!("entity#{}", e.0)).collect::<Vec<_>>().join(", "),
            _ => "(mixed)".to_string(),
        };
        println!("  cohere#{} [{ms}] strength={:.3}", c.id.0, c.strength);
    }
    println!("\nDone.");
}
