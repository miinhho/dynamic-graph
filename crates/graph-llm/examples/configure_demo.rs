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
struct RefConfig {
    label: &'static str,
    description: &'static str,
    ref_decay: f32,
    ref_contrib: f32,
    ref_lr: f32,
}

#[cfg(feature = "ollama")]
struct Noop;

#[cfg(feature = "ollama")]
impl graph_core::LocusProgram for Noop {
    fn process(
        &self,
        _: &graph_core::Locus,
        _: &[&graph_core::Change],
        _: &dyn graph_core::LocusContext,
    ) -> Vec<graph_core::ProposedChange> {
        vec![]
    }
}

#[cfg(feature = "ollama")]
fn main() {
    use graph_llm::OllamaClient;

    let model = std::env::var("OLLAMA_MODEL").unwrap_or_else(|_| "llama3:8b".to_owned());
    let client = OllamaClient::new(&model);
    println!("=== LLM-assisted parameter configuration  (model: {model}) ===\n");

    run_influence_domain(
        &client,
        "Domain 1: Neuroscience (C. elegans connectome)",
        &neuroscience_cases(),
    );
    run_influence_domain(
        &client,
        "Domain 2: Social Network (rumor & tie strength)",
        &social_cases(),
    );
    run_influence_domain(
        &client,
        "Domain 3: Supply Chain (demand propagation)",
        &supply_cases(),
    );
    run_threshold_calibration(&client);
    run_end_to_end_social_simulation(&client);
    println!("\n=== done ===");
}

#[cfg(feature = "ollama")]
fn neuroscience_cases() -> [RefConfig; 3] {
    [
        RefConfig {
            label: "excitatory synapse",
            description: "Excitatory chemical synapse (glutamate): activity fades moderately \
                between ticks (retains ~85% per tick), strongly positive contribution per \
                spike, saturates at moderate levels, and pathways strengthen through \
                repeated co-firing (Hebbian plasticity, fast learning rate ~0.5).",
            ref_decay: 0.85,
            ref_contrib: 1.0,
            ref_lr: 0.5,
        },
        RefConfig {
            label: "inhibitory synapse",
            description: "Inhibitory chemical synapse (GABA): same moderate decay as \
                excitatory synapses (~85% per tick), but each spike REDUCES activity \
                by a similar magnitude (negative contribution). No plasticity.",
            ref_decay: 0.85,
            ref_contrib: -1.0,
            ref_lr: 0.0,
        },
        RefConfig {
            label: "sensory input",
            description: "Sensory neuron input: completely persistent, no decay \
                (sensory signals represent current environmental state, not transient \
                spikes). Mildly positive contribution per event.",
            ref_decay: 1.0,
            ref_contrib: 1.0,
            ref_lr: 0.0,
        },
    ]
}

#[cfg(feature = "ollama")]
fn social_cases() -> [RefConfig; 3] {
    [
        RefConfig {
            label: "viral rumor",
            description: "Viral rumor spread: decays very quickly (rumors lose credibility \
                fast — retains only ~60% per tick), strong positive activity per share. \
                No plasticity (repeat sharing doesn't strengthen the channel long-term).",
            ref_decay: 0.6,
            ref_contrib: 2.0,
            ref_lr: 0.0,
        },
        RefConfig {
            label: "weak social tie",
            description: "Weak social tie (acquaintance): very slow decay (connections \
                persist for years — retains ~98% per tick), mild positive contribution \
                per interaction, slight Hebbian plasticity (lr ~0.01) as occasional \
                interactions slowly strengthen the bond.",
            ref_decay: 0.98,
            ref_contrib: 0.3,
            ref_lr: 0.01,
        },
        RefConfig {
            label: "strong friendship",
            description: "Strong friendship: nearly permanent (very slow decay ~99% \
                retention), high contribution per interaction, significant Hebbian \
                plasticity (lr ~0.1) — regular contact strongly reinforces the \
                relationship weight over time.",
            ref_decay: 0.99,
            ref_contrib: 1.0,
            ref_lr: 0.1,
        },
    ]
}

#[cfg(feature = "ollama")]
fn supply_cases() -> [RefConfig; 2] {
    [
        RefConfig {
            label: "demand signal",
            description: "Customer demand signal: decays moderately fast between restocking \
                cycles (~75% retained per tick), strong positive contribution per order, \
                no learning (demand signals don't strengthen supplier relationships).",
            ref_decay: 0.75,
            ref_contrib: 1.5,
            ref_lr: 0.0,
        },
        RefConfig {
            label: "inventory shock",
            description: "Inventory disruption (stockout/overstock): very fast decay \
                (~40% retained — shocks are temporary), large negative contribution \
                (shortages cascade negatively upstream). Only creates a relationship \
                if the shock is substantial.",
            ref_decay: 0.4,
            ref_contrib: -2.0,
            ref_lr: 0.0,
        },
    ]
}

#[cfg(feature = "ollama")]
fn run_influence_domain(client: &graph_llm::OllamaClient, title: &str, cases: &[RefConfig]) {
    println!("━━━ {title} ━━━\n");
    for case in cases {
        print_influence_case(client, case);
    }
}

#[cfg(feature = "ollama")]
fn run_threshold_calibration(client: &graph_llm::OllamaClient) {
    println!("━━━ Domain 4: Emergence & Cohere thresholds ━━━\n");

    let emergence_cases: &[(&str, f32)] = &[
        (
            "Include even faint synaptic connections in community detection.",
            0.05,
        ),
        (
            "Only detect communities where edges are strongly active.",
            0.25,
        ),
    ];
    for (description, reference) in emergence_cases {
        print_emergence_case(client, description, *reference);
    }

    let cohere_cases: &[(&str, f32)] = &[
        (
            "Merge entity clusters only when they are strongly bridged — ignore faint cross-entity links.",
            0.3,
        ),
        (
            "Group entities together even if they share only a faint connection.",
            0.05,
        ),
    ];
    for (description, reference) in cohere_cases {
        print_cohere_case(client, description, *reference);
    }
}

#[cfg(feature = "ollama")]
fn run_end_to_end_social_simulation(client: &graph_llm::OllamaClient) {
    println!("━━━ End-to-end: social network with LLM-inferred params ━━━\n");

    let rumor_cfg = infer_rumor_config(client);
    let friend_cfg = infer_friendship_config(client);
    let mut sim = build_social_simulation(&rumor_cfg, &friend_cfg);

    print_inferred_configs(&rumor_cfg, &friend_cfg);
    seed_social_graph(&mut sim);
    print_relationship_count(&sim, "\n  After co-occurrence flush");
    run_decay_ticks(&mut sim, 10);
    print_relationship_count(&sim, "  After 10 decay ticks    ");
    print_decay_summary(&rumor_cfg, &friend_cfg, 10);
}

#[cfg(feature = "ollama")]
fn infer_rumor_config(client: &graph_llm::OllamaClient) -> graph_engine::InfluenceKindConfig {
    use graph_engine::InfluenceKindConfig;
    use graph_llm::configure_influence;

    configure_influence(
        client,
        "rumor",
        "Viral rumor: decays very quickly (half-life of ~1 tick), \
         strong positive contribution, no plasticity.",
    )
    .unwrap_or_else(|e| {
        eprintln!("  [fallback rumor] {e}");
        InfluenceKindConfig::new("rumor")
            .with_decay(0.6)
            .with_activity_contribution(2.0)
    })
}

#[cfg(feature = "ollama")]
fn infer_friendship_config(client: &graph_llm::OllamaClient) -> graph_engine::InfluenceKindConfig {
    use graph_engine::{InfluenceKindConfig, PlasticityConfig};
    use graph_llm::configure_influence;

    configure_influence(
        client,
        "friendship",
        "Strong friendship bond: extremely slow decay (persists for hundreds of ticks), \
         moderate positive contribution, moderate Hebbian plasticity.",
    )
    .unwrap_or_else(|e| {
        eprintln!("  [fallback friendship] {e}");
        InfluenceKindConfig::new("friendship")
            .with_decay(0.99)
            .with_activity_contribution(1.0)
            .with_plasticity(PlasticityConfig {
                learning_rate: 0.05,
                weight_decay: 0.999,
                max_weight: 2.0,
                ..Default::default()
            })
    })
}

#[cfg(feature = "ollama")]
fn build_social_simulation(
    rumor_cfg: &graph_engine::InfluenceKindConfig,
    friend_cfg: &graph_engine::InfluenceKindConfig,
) -> graph_engine::Simulation {
    use graph_engine::{PlasticityConfig, SimulationBuilder};

    let rumor_decay = rumor_cfg.decay_per_batch;
    let rumor_contrib = rumor_cfg.activity_contribution;
    let friend_decay = friend_cfg.decay_per_batch;
    let friend_contrib = friend_cfg.activity_contribution;
    let friend_lr = friend_cfg.plasticity.learning_rate;
    let friend_wd = friend_cfg.plasticity.weight_decay;
    let friend_mw = friend_cfg.plasticity.max_weight;

    SimulationBuilder::new()
        .locus_kind("person", Noop)
        .default_influence("rumor")
        .influence("rumor", move |cfg| {
            cfg.with_decay(rumor_decay)
                .with_activity_contribution(rumor_contrib)
                .symmetric()
        })
        .influence("friendship", move |cfg| {
            cfg.with_decay(friend_decay)
                .with_activity_contribution(friend_contrib)
                .with_plasticity(PlasticityConfig {
                    learning_rate: friend_lr,
                    weight_decay: friend_wd,
                    max_weight: friend_mw,
                    ..Default::default()
                })
                .symmetric()
        })
        .build()
}

#[cfg(feature = "ollama")]
fn print_inferred_configs(
    rumor_cfg: &graph_engine::InfluenceKindConfig,
    friend_cfg: &graph_engine::InfluenceKindConfig,
) {
    println!(
        "  Inferred rumor     : decay={:.3}  contrib={:+.2}  lr=0.000",
        rumor_cfg.decay_per_batch, rumor_cfg.activity_contribution
    );
    println!(
        "  Inferred friendship: decay={:.3}  contrib={:+.2}  lr={:.3}",
        friend_cfg.decay_per_batch,
        friend_cfg.activity_contribution,
        friend_cfg.plasticity.learning_rate
    );
}

#[cfg(feature = "ollama")]
fn seed_social_graph(sim: &mut graph_engine::Simulation) {
    use graph_core::props;

    sim.ingest_cooccurrence(vec![
        ("alice", "person", props! {}),
        ("bob", "person", props! {}),
        ("carol", "person", props! {}),
    ]);
    sim.step(vec![]);
}

#[cfg(feature = "ollama")]
fn run_decay_ticks(sim: &mut graph_engine::Simulation, ticks: usize) {
    for _ in 0..ticks {
        sim.step(vec![]);
    }
}

#[cfg(feature = "ollama")]
fn print_relationship_count(sim: &graph_engine::Simulation, label: &str) {
    let world = sim.world();
    println!("{label}: {} relationships", world.relationships().len());
}

#[cfg(feature = "ollama")]
fn print_decay_summary(
    rumor_cfg: &graph_engine::InfluenceKindConfig,
    friend_cfg: &graph_engine::InfluenceKindConfig,
    ticks: i32,
) {
    let rumor_remaining = rumor_cfg.decay_per_batch.powi(ticks);
    let friend_remaining = friend_cfg.decay_per_batch.powi(ticks);
    let decay_ordering_ok = rumor_cfg.decay_per_batch < friend_cfg.decay_per_batch;
    let meaningful_spread = friend_remaining > rumor_remaining * 2.0;

    println!("\n  Expected remaining activity after {ticks} ticks (starting from 1.0):");
    println!("    rumor:      {rumor_remaining:.4}");
    println!("    friendship: {friend_remaining:.4}");
    println!(
        "    spread:     {:.4}  {}",
        friend_remaining - rumor_remaining,
        if meaningful_spread {
            "✓ meaningful difference"
        } else {
            "✗ too similar"
        }
    );
    println!(
        "  Decay ordering (rumor < friendship): {decay_ordering_ok}  {}",
        if decay_ordering_ok { "✓" } else { "✗" }
    );
}

#[cfg(feature = "ollama")]
fn print_emergence_case(client: &graph_llm::OllamaClient, description: &str, ref_activity: f32) {
    use graph_llm::configure_emergence;

    println!(
        "  ▶ emergence: \"{}...\"",
        &description[..description.len().min(70)]
    );
    match configure_emergence(client, description) {
        Ok(params) => {
            println!("    LLM → min_activity={:?}", params.min_activity_threshold);
            println!("    REF → min_activity={:.3}", ref_activity);
            let activity_ok = params
                .min_activity_threshold
                .map(|value| (value - ref_activity).abs() < 0.15)
                .unwrap_or(false);
            println!(
                "    {}  (activity_close={activity_ok})",
                if activity_ok {
                    "✓ reasonable"
                } else {
                    "✗ diverged"
                }
            );
        }
        Err(error) => println!("    [error] {error}"),
    }
    println!();
}

#[cfg(feature = "ollama")]
fn print_cohere_case(client: &graph_llm::OllamaClient, description: &str, ref_bridge: f32) {
    use graph_llm::configure_cohere;

    println!(
        "  ▶ cohere: \"{}\"",
        &description[..description.len().min(70)]
    );
    match configure_cohere(client, description) {
        Ok(params) => {
            println!("    LLM → min_bridge={:?}", params.min_bridge_activity);
            println!("    REF → min_bridge={:.3}", ref_bridge);
            let direction_ok = params
                .min_bridge_activity
                .map(|value| {
                    if ref_bridge >= 0.2 {
                        value >= 0.15
                    } else {
                        value <= 0.15
                    }
                })
                .unwrap_or(false);
            println!(
                "    {}  (direction_ok={direction_ok})",
                if direction_ok {
                    "✓ reasonable"
                } else {
                    "✗ diverged"
                }
            );
        }
        Err(error) => println!("    [error] {error}"),
    }
    println!();
}

#[cfg(feature = "ollama")]
fn print_influence_case(client: &graph_llm::OllamaClient, case: &RefConfig) {
    use graph_llm::configure_influence;

    println!("  ▶ {}", case.label);
    match configure_influence(client, case.label, case.description) {
        Ok(cfg) => {
            println!(
                "    LLM → decay={:.3}  contrib={:+.2}  lr={:.3}",
                cfg.decay_per_batch, cfg.activity_contribution, cfg.plasticity.learning_rate
            );
            println!(
                "    REF → decay={:.3}  contrib={:+.2}  lr={:.3}",
                case.ref_decay, case.ref_contrib, case.ref_lr
            );
            let decay_ok = (cfg.decay_per_batch - case.ref_decay).abs() < 0.15;
            let sign_ok = cfg.activity_contribution.signum() == case.ref_contrib.signum();
            let lr_ok = if case.ref_lr == 0.0 {
                cfg.plasticity.learning_rate < 0.05
            } else {
                cfg.plasticity.learning_rate > 0.0
            };
            println!(
                "    {}  (decay_close={decay_ok}  sign_ok={sign_ok}  lr_ok={lr_ok})",
                if decay_ok && sign_ok && lr_ok {
                    "✓ reasonable"
                } else {
                    "✗ diverged"
                }
            );
        }
        Err(e) => println!("    [error] {e}"),
    }
    println!();
}
