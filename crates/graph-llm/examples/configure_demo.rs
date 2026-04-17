//! Demonstrate LLM-assisted parameter configuration against real domain scenarios.
//!
//! Tests three domains:
//!   1. Neuroscience (excitatory / inhibitory / sensory synapses)
//!   2. Social network (rumor spread / weak tie / strong friendship)
//!   3. Supply chain (demand signal / inventory shock)
//!
//! For each, the LLM infers numeric parameters from plain-language descriptions.
//! The end-to-end section shows those params running in a real simulation.
//!
//! Run with:
//!   cargo run -p graph-llm --example configure_demo --features ollama

#[cfg(not(feature = "ollama"))]
fn main() {
    eprintln!("Run with: cargo run -p graph-llm --example configure_demo --features ollama");
}

#[cfg(feature = "ollama")]
fn main() {
    use graph_core::{Change, Locus, LocusContext, LocusProgram, ProposedChange, props};
    use graph_engine::{InfluenceKindConfig, PlasticityConfig, SimulationBuilder};
    #[allow(unused_imports)]
    use graph_core::ProposedChange as _ProposedChange;
    use graph_llm::{OllamaClient, configure_cohere, configure_emergence, configure_influence};

    let model = std::env::var("OLLAMA_MODEL").unwrap_or_else(|_| "llama3:8b".to_owned());
    let client = OllamaClient::new(&model);
    println!("=== LLM-assisted parameter configuration  (model: {model}) ===\n");

    // ─────────────────────────────────────────────────────────────────────────
    // Domain 1: Neuroscience — C. elegans-style connectome
    // Reference values from examples/celegans.rs (hand-tuned)
    // ─────────────────────────────────────────────────────────────────────────
    println!("━━━ Domain 1: Neuroscience (C. elegans connectome) ━━━\n");

    struct RefConfig {
        label:       &'static str,
        description: &'static str,
        ref_decay:   f32,
        ref_contrib: f32,
        ref_lr:      f32,
    }

    let neuro_cases = [
        RefConfig {
            label: "excitatory synapse",
            description: "Excitatory chemical synapse (glutamate): activity fades moderately \
                between ticks (retains ~85% per tick), strongly positive contribution per \
                spike, saturates at moderate levels, and pathways strengthen through \
                repeated co-firing (Hebbian plasticity, fast learning rate ~0.5).",
            ref_decay: 0.85, ref_contrib: 1.0, ref_lr: 0.5,
        },
        RefConfig {
            label: "inhibitory synapse",
            description: "Inhibitory chemical synapse (GABA): same moderate decay as \
                excitatory synapses (~85% per tick), but each spike REDUCES activity \
                by a similar magnitude (negative contribution). No plasticity.",
            ref_decay: 0.85, ref_contrib: -1.0, ref_lr: 0.0,
        },
        RefConfig {
            label: "sensory input",
            description: "Sensory neuron input: completely persistent, no decay \
                (sensory signals represent current environmental state, not transient \
                spikes). Mildly positive contribution per event.",
            ref_decay: 1.0, ref_contrib: 1.0, ref_lr: 0.0,
        },
    ];

    for r in &neuro_cases {
        print_influence_case(&client, r.label, r.description, r.ref_decay, r.ref_contrib, r.ref_lr);
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Domain 2: Social network — rumor and tie strength dynamics
    // ─────────────────────────────────────────────────────────────────────────
    println!("━━━ Domain 2: Social Network (rumor & tie strength) ━━━\n");

    let social_cases = [
        RefConfig {
            label: "viral rumor",
            description: "Viral rumor spread: decays very quickly (rumors lose credibility \
                fast — retains only ~60% per tick), strong positive activity per share. \
                No plasticity (repeat sharing doesn't strengthen the channel long-term).",
            ref_decay: 0.6, ref_contrib: 2.0, ref_lr: 0.0,
        },
        RefConfig {
            label: "weak social tie",
            description: "Weak social tie (acquaintance): very slow decay (connections \
                persist for years — retains ~98% per tick), mild positive contribution \
                per interaction, slight Hebbian plasticity (lr ~0.01) as occasional \
                interactions slowly strengthen the bond.",
            ref_decay: 0.98, ref_contrib: 0.3, ref_lr: 0.01,
        },
        RefConfig {
            label: "strong friendship",
            description: "Strong friendship: nearly permanent (very slow decay ~99% \
                retention), high contribution per interaction, significant Hebbian \
                plasticity (lr ~0.1) — regular contact strongly reinforces the \
                relationship weight over time.",
            ref_decay: 0.99, ref_contrib: 1.0, ref_lr: 0.1,
        },
    ];

    for r in &social_cases {
        print_influence_case(&client, r.label, r.description, r.ref_decay, r.ref_contrib, r.ref_lr);
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Domain 3: Supply chain — demand propagation
    // ─────────────────────────────────────────────────────────────────────────
    println!("━━━ Domain 3: Supply Chain (demand propagation) ━━━\n");

    let supply_cases = [
        RefConfig {
            label: "demand signal",
            description: "Customer demand signal: decays moderately fast between restocking \
                cycles (~75% retained per tick), strong positive contribution per order, \
                no learning (demand signals don't strengthen supplier relationships).",
            ref_decay: 0.75, ref_contrib: 1.5, ref_lr: 0.0,
        },
        RefConfig {
            label: "inventory shock",
            description: "Inventory disruption (stockout/overstock): very fast decay \
                (~40% retained — shocks are temporary), large negative contribution \
                (shortages cascade negatively upstream). Only creates a relationship \
                if the shock is substantial.",
            ref_decay: 0.4, ref_contrib: -2.0, ref_lr: 0.0,
        },
    ];

    for r in &supply_cases {
        print_influence_case(&client, r.label, r.description, r.ref_decay, r.ref_contrib, r.ref_lr);
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Domain 4: Emergence & Cohere threshold calibration
    // ─────────────────────────────────────────────────────────────────────────
    println!("━━━ Domain 4: Emergence & Cohere thresholds ━━━\n");

    let emergence_cases: &[(&str, f32, f32)] = &[
        (
            "Include even faint synaptic connections in community detection. \
             Entities can shift membership significantly between ticks.",
            0.05, 0.4,
        ),
        (
            "Only detect communities where edges are strongly active. \
             Require high membership overlap before merging a component into an existing entity.",
            0.25, 0.7,
        ),
    ];

    for (desc, ref_activity, ref_overlap) in emergence_cases {
        println!("  ▶ emergence: \"{}...\"", &desc[..desc.len().min(70)]);
        match configure_emergence(&client, desc) {
            Ok(p) => {
                println!("    LLM → min_activity={:.3}  overlap={:.3}", p.min_activity_threshold, p.overlap_threshold);
                println!("    REF → min_activity={:.3}  overlap={:.3}", ref_activity, ref_overlap);
                let a_ok = (p.min_activity_threshold - ref_activity).abs() < 0.15;
                let o_ok = (p.overlap_threshold - ref_overlap).abs() < 0.2;
                println!("    {}  (activity_close={a_ok}  overlap_close={o_ok})",
                    if a_ok && o_ok { "✓ reasonable" } else { "✗ diverged" });
            }
            Err(e) => println!("    [error] {e}"),
        }
        println!();
    }

    let cohere_cases: &[(&str, f32)] = &[
        ("Merge entity clusters only when they are strongly bridged — ignore faint cross-entity links.", 0.3),
        ("Group entities together even if they share only a faint connection.", 0.05),
    ];

    for (desc, ref_bridge) in cohere_cases {
        println!("  ▶ cohere: \"{}\"", &desc[..desc.len().min(70)]);
        match configure_cohere(&client, desc) {
            Ok(p) => {
                println!("    LLM → min_bridge={:.3}", p.min_bridge_activity);
                println!("    REF → min_bridge={:.3}", ref_bridge);
                let direction_ok = if *ref_bridge >= 0.2 {
                    p.min_bridge_activity >= 0.15
                } else {
                    p.min_bridge_activity <= 0.15
                };
                println!("    {}  (direction_ok={direction_ok})",
                    if direction_ok { "✓ reasonable" } else { "✗ diverged" });
            }
            Err(e) => println!("    [error] {e}"),
        }
        println!();
    }

    // ─────────────────────────────────────────────────────────────────────────
    // End-to-end: social network simulation with LLM-inferred configs
    // Two influence kinds: rumor (fast decay) and friendship (slow decay).
    // Verifiable property: after N ticks with no new stimuli,
    //   rumor activity should decay much more than friendship activity.
    // ─────────────────────────────────────────────────────────────────────────
    println!("━━━ End-to-end: social network with LLM-inferred params ━━━\n");

    struct Noop;
    impl LocusProgram for Noop {
        fn process(&self, _: &Locus, _: &[&Change], _: &dyn LocusContext) -> Vec<ProposedChange> {
            vec![]
        }
    }

    let rumor_cfg = configure_influence(&client, "rumor",
        "Viral rumor: decays very quickly (half-life of ~1 tick), \
         strong positive contribution, no plasticity.")
        .unwrap_or_else(|e| {
            eprintln!("  [fallback rumor] {e}");
            InfluenceKindConfig::new("rumor").with_decay(0.6).with_activity_contribution(2.0)
        });

    let friend_cfg = configure_influence(&client, "friendship",
        "Strong friendship bond: extremely slow decay (persists for hundreds of ticks), \
         moderate positive contribution, moderate Hebbian plasticity.")
        .unwrap_or_else(|e| {
            eprintln!("  [fallback friendship] {e}");
            InfluenceKindConfig::new("friendship")
                .with_decay(0.99).with_activity_contribution(1.0)
                .with_plasticity(PlasticityConfig { learning_rate: 0.05, weight_decay: 0.999, max_weight: 2.0, stdp: false,
            ..Default::default() })
        });

    let rumor_decay    = rumor_cfg.decay_per_batch;
    let friend_decay   = friend_cfg.decay_per_batch;
    let rumor_contrib  = rumor_cfg.activity_contribution;
    let friend_contrib = friend_cfg.activity_contribution;
    let friend_lr      = friend_cfg.plasticity.learning_rate;
    let friend_wd      = friend_cfg.plasticity.weight_decay;
    let friend_mw      = friend_cfg.plasticity.max_weight;

    println!("  Inferred rumor     : decay={rumor_decay:.3}  contrib={rumor_contrib:+.2}  lr=0.000");
    println!("  Inferred friendship: decay={friend_decay:.3}  contrib={friend_contrib:+.2}  lr={friend_lr:.3}");

    let mut sim = SimulationBuilder::new()
        .locus_kind("person", Noop)
        .default_influence("rumor")
        .influence("rumor", move |cfg| cfg
            .with_decay(rumor_decay)
            .with_activity_contribution(rumor_contrib)
            .symmetric()
        )
        .influence("friendship", move |cfg| cfg
            .with_decay(friend_decay)
            .with_activity_contribution(friend_contrib)
            .with_plasticity(PlasticityConfig { learning_rate: friend_lr, weight_decay: friend_wd, max_weight: friend_mw, stdp: false,
            ..Default::default() })
            .symmetric()
        )
        .build();

    // Create three nodes; co-occurrence auto-wires relationships
    sim.ingest_cooccurrence(vec![
        ("alice", "person", props! {}),
        ("bob",   "person", props! {}),
        ("carol", "person", props! {}),
    ]);
    sim.step(vec![]);  // flush → relationships emerge

    let world = sim.world();
    println!("\n  After co-occurrence flush: {} relationships", world.relationships().len());
    drop(world);

    // Run 10 decay ticks
    for _ in 0..10 { sim.step(vec![]); }

    let world = sim.world();
    println!("  After 10 decay ticks     : {} relationships", world.relationships().len());
    drop(world);

    // Key property: decay ordering should be correct
    let decay_ordering_ok = rumor_decay < friend_decay;
    let rumor_remaining   = rumor_decay.powi(10);
    let friend_remaining  = friend_decay.powi(10);
    let meaningful_spread = friend_remaining > rumor_remaining * 2.0;

    println!("\n  Expected remaining activity after 10 ticks (starting from 1.0):");
    println!("    rumor:      {rumor_remaining:.4}");
    println!("    friendship: {friend_remaining:.4}");
    println!("    spread:     {:.4}  {}",
        friend_remaining - rumor_remaining,
        if meaningful_spread { "✓ meaningful difference" } else { "✗ too similar" });
    println!("  Decay ordering (rumor < friendship): {decay_ordering_ok}  {}",
        if decay_ordering_ok { "✓" } else { "✗" });

    println!("\n=== done ===");
}

#[cfg(feature = "ollama")]
fn print_influence_case(
    client: &graph_llm::OllamaClient,
    label: &str,
    description: &str,
    ref_decay: f32,
    ref_contrib: f32,
    ref_lr: f32,
) {
    use graph_llm::configure_influence;
    println!("  ▶ {label}");
    match configure_influence(client, label, description) {
        Ok(cfg) => {
            println!("    LLM → decay={:.3}  contrib={:+.2}  lr={:.3}",
                cfg.decay_per_batch, cfg.activity_contribution, cfg.plasticity.learning_rate);
            println!("    REF → decay={:.3}  contrib={:+.2}  lr={:.3}",
                ref_decay, ref_contrib, ref_lr);
            let decay_ok  = (cfg.decay_per_batch - ref_decay).abs() < 0.15;
            let sign_ok   = cfg.activity_contribution.signum() == ref_contrib.signum();
            let lr_ok     = if ref_lr == 0.0 {
                cfg.plasticity.learning_rate < 0.05
            } else {
                cfg.plasticity.learning_rate > 0.0
            };
            println!("    {}  (decay_close={decay_ok}  sign_ok={sign_ok}  lr_ok={lr_ok})",
                if decay_ok && sign_ok && lr_ok { "✓ reasonable" } else { "✗ diverged" });
        }
        Err(e) => println!("    [error] {e}"),
    }
    println!();
}
