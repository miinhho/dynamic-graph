//! Smoke-test the three graph-llm capabilities against a local Ollama server.
//!
//! Requires Ollama running at http://localhost:11434 with a model installed.
//! Default model: llama3:8b — override with OLLAMA_MODEL env var.
//!
//! ```
//! cargo run -p graph-llm --example ollama_smoke --features ollama
//! ```

#[cfg(not(feature = "ollama"))]
fn main() {
    eprintln!("Run with: cargo run -p graph-llm --example ollama_smoke --features ollama");
}

#[cfg(feature = "ollama")]
fn main() {
    use graph_boundary::{analyze_boundary, prescribe_updates, PrescriptionConfig};
    use graph_core::{Change, Locus, LocusContext, LocusProgram, ProposedChange, props};
    use graph_engine::{InfluenceKindConfig, SimulationBuilder};
    use graph_llm::{
        GraphLlm, OllamaClient, TextIngestor,
        answer_with_graph,
        narrate_counterfactual, narrate_entity_deviations, narrate_prescriptions,
    };
    use graph_query::{entity_deviations_since, NameMap, relationships_absent_without};
    use graph_schema::SchemaWorld;
    use graph_core::BatchId;

    struct Noop;
    impl LocusProgram for Noop {
        fn process(&self, _: &Locus, _: &[&Change], _: &dyn LocusContext) -> Vec<ProposedChange> {
            vec![]
        }
    }

    let model = std::env::var("OLLAMA_MODEL").unwrap_or_else(|_| "llama3:8b".to_owned());
    let client = OllamaClient::new(&model);
    println!("=== Ollama smoke test  (model: {model}) ===\n");

    // ── 1. Text ingestion ───────────────────────────────────────────────────────
    println!("── 1. Text → Entities ─────────────────────────────────────────────");
    let text = "Marie Curie was a physicist and chemist at the University of Paris. \
                She collaborated with Pierre Curie and Henri Becquerel on radioactivity.";
    let ingestor = TextIngestor::new(&client);
    match ingestor.extract(text, &["PERSON", "ORG"]) {
        Ok(nodes) => {
            println!("Extracted {} node(s):", nodes.len());
            for n in &nodes {
                println!("  [{:>8}] {}", n.kind, n.name);
                for (k, v) in &n.properties {
                    println!("           {k}: {v}");
                }
            }
        }
        Err(e) => println!("  [ingestion error] {e}"),
    }

    // ── 2. Build a small world for narration tests ──────────────────────────────
    let mut sim = SimulationBuilder::new()
        .locus_kind("NODE", Noop)
        .influence("signal", |cfg: InfluenceKindConfig| cfg.with_decay(0.9).symmetric())
        .default_influence("signal")
        .build();

    // First co-occurrence: creates the loci.
    sim.ingest_cooccurrence(vec![
        ("alice", "NODE", props! { "name" => "alice" }),
        ("bob",   "NODE", props! { "name" => "bob"   }),
        ("carol", "NODE", props! { "name" => "carol" }),
    ]);

    // Second co-occurrence: wires cross-locus predecessors → relationships emerge.
    sim.ingest_cooccurrence(vec![
        ("alice", "NODE", props! { "name" => "alice" }),
        ("bob",   "NODE", props! { "name" => "bob"   }),
        ("carol", "NODE", props! { "name" => "carol" }),
    ]);

    // Third co-occurrence: different subset → dave added.
    sim.ingest_cooccurrence(vec![
        ("bob",  "NODE", props! { "name" => "bob"  }),
        ("dave", "NODE", props! { "name" => "dave" }),
    ]);

    let world = &sim.world;
    let names = NameMap::from_world(world);
    println!("\nWorld: {} loci, {} relationships", world.loci().len(), world.relationships().len());

    // ── 2. Counterfactual narration ─────────────────────────────────────────────
    println!("\n── 2. Counterfactual narration ────────────────────────────────────");

    let batch0_changes: Vec<_> = world
        .log()
        .batch(BatchId(0))
        .map(|c| c.id)
        .collect();

    let absent = relationships_absent_without(world, &batch0_changes);
    let name_pairs: Vec<(String, String)> = absent
        .iter()
        .filter_map(|&id| world.relationships().get(id))
        .map(|rel: &graph_core::Relationship| {
            let (a, b) = match rel.endpoints {
                graph_core::Endpoints::Symmetric { a, b } => (a, b),
                graph_core::Endpoints::Directed { from, to } => (from, to),
            };
            (names.name(a), names.name(b))
        })
        .collect();

    println!("Relationships absent without batch-0 stimuli: {}", name_pairs.len());
    match narrate_counterfactual(&client, &name_pairs) {
        Ok(prose) => println!("\nNarration:\n{prose}"),
        Err(e)    => println!("  [error] {e}"),
    }

    // ── 3. Entity deviation narration ───────────────────────────────────────────
    println!("\n── 3. Entity deviation narration ──────────────────────────────────");

    let diffs = entity_deviations_since(world, BatchId(0));
    println!("Entity diffs since batch 0: {}", diffs.len());

    match narrate_entity_deviations(&client, &diffs, &names) {
        Ok(prose) => println!("\nNarration:\n{prose}"),
        Err(e)    => println!("  [error] {e}"),
    }

    // ── 4. Schema tension narration ─────────────────────────────────────────────
    println!("\n── 4. Schema tension narration ────────────────────────────────────");

    let schema  = SchemaWorld::new();
    let report  = analyze_boundary(world, &schema, None);
    let actions = prescribe_updates(&report, &schema, world, &PrescriptionConfig::default());
    println!("Boundary actions: {}", actions.len());

    match narrate_prescriptions(&client, &actions, &schema, &names) {
        Ok(prose) => println!("\nNarration:\n{prose}"),
        Err(e)    => println!("  [error] {e}"),
    }

    // ── 5. Graph-grounded Q&A ───────────────────────────────────────────────────
    println!("\n── 5. Graph-grounded Q&A ──────────────────────────────────────────");
    let questions = [
        "Who is alice connected to?",
        "What do we know about dave?",
        "Who is charlie?",   // 그래프에 없는 이름 — context 없이 전달됨
    ];
    for q in &questions {
        println!("\nQ: {q}");
        match answer_with_graph(&client, q, world, &names, 5) {
            Ok(answer) => println!("A: {answer}"),
            Err(e)     => println!("  [error] {e}"),
        }
    }

    // ── 6. Facade API ───────────────────────────────────────────────────────────
    println!("\n── 6. Facade API (GraphLlm) ───────────────────────────────────────");
    let g = GraphLlm::new(&client, world).with_top_k(5);

    println!("Q: Who is bob connected to?");
    match g.ask("Who is bob connected to?") {
        Ok(answer) => println!("A: {answer}"),
        Err(e)     => println!("  [error] {e}"),
    }

    println!("\n=== done ===");
}
