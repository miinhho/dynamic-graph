//! Knowledge-graph co-occurrence example.
//!
//! Builds an entity co-occurrence graph from a stream of "documents". Each
//! document is a set of named entities that appear together. We model
//! co-occurrence as causal signal flow: entity A's change is set as a
//! predecessor of entity B's stimulus, so the engine auto-emerges a
//! relationship between them.
//!
//! Demonstrates:
//!
//! 1. **`ingest` API** — entities are created by name with domain
//!    properties (`name`, `type`); the simulation handles locus allocation,
//!    property storage, and name resolution.
//!
//! 2. **`LocusContext::properties(id)`** — the entity program reads
//!    neighbor names through the context interface (no direct world access).
//!
//! 3. **`inbox::of_kind` + `inbox::locus_signals`** — sum incoming
//!    co-occurrence signal with zero boilerplate.
//!
//! 4. **`graph_query`** — connected components, shortest paths, and
//!    reachability queries on the emerged topology.
//!
//! Co-occurrence graph after all documents:
//!
//! ```text
//!   Apple ─── Tim_Cook ─── Tesla
//!     │                      │
//!   OpenAI ───────────── Elon_Musk
//! ```
//!
//! Run: `cargo run -p graph-engine --example knowledge_graph`

use graph_core::{
    Change, ChangeId, InfluenceKindId, Locus, LocusContext, LocusId, LocusProgram, ProposedChange,
    SaturationMode, StabilizationConfig,
    inbox::{locus_signals, of_kind},
    props,
};
use graph_engine::{InfluenceKindConfig, PlasticityConfig, SimulationBuilder};
use graph_query::{connected_components, path_between, reachable_from};

// ── Kind IDs ──────────────────────────────────────────────────────────────────

/// Co-occurrence influence kind.
const COOCCUR: InfluenceKindId = InfluenceKindId(1);

// ── Programs ──────────────────────────────────────────────────────────────────

/// Program for every entity locus (ORG or PERSON).
///
/// On a co-occurrence stimulus: propagate half the signal to all currently
/// connected neighbors. Uses `ctx.properties(id)` to log neighbor names;
/// uses `inbox::of_kind` + `inbox::locus_signals` to compute the net signal.
struct EntityProgram;

impl LocusProgram for EntityProgram {
    fn process(
        &self,
        locus: &Locus,
        incoming: &[&Change],
        ctx: &dyn LocusContext,
    ) -> Vec<ProposedChange> {
        let signal = locus_signals(of_kind(incoming, COOCCUR));
        if signal < 1e-6 {
            return vec![];
        }

        let my_name = ctx
            .properties(locus.id)
            .and_then(|p| p.get_str("name"))
            .unwrap_or("<unknown>");

        ctx.relationships_for(locus.id)
            .map(|rel| {
                let neighbor_id = rel.endpoints.other_than(locus.id);
                let neighbor_name = ctx
                    .properties(neighbor_id)
                    .and_then(|p| p.get_str("name"))
                    .unwrap_or("<unknown>");
                let weight = rel.state.as_slice().get(1).copied().unwrap_or(0.0);
                eprintln!(
                    "  [{my_name}] → [{neighbor_name}]  signal={signal:.3}  weight={weight:.3}"
                );
                ProposedChange::activation(neighbor_id, COOCCUR, signal * 0.4)
            })
            .collect()
    }
}

// ── Co-occurrence helper ───────────────────────────────────────────────────────

/// Ingest a document: a list of entity `(name, kind_str, properties)` tuples
/// that co-occur in the same document.
///
/// For each pair (A, B), we create a stimulus for B that declares A's most
/// recent change as a predecessor. This causes the engine to auto-emerge a
/// co-occurrence relationship between A and B.
///
/// Returns the `LocusId`s in the same order as `entries`.
fn ingest_document(
    sim: &mut graph_engine::Simulation,
    entries: Vec<(&str, &str, graph_core::Properties)>,
) -> Vec<LocusId> {
    // First, resolve/create all entity loci.
    let ids: Vec<LocusId> = entries
        .into_iter()
        .map(|(name, kind, props)| sim.ingest_named(name, kind, props))
        .collect();

    // Find the most recent ChangeId for each entity (may be None for brand-new loci
    // that have never been stepped yet).
    let last_change: Vec<Option<ChangeId>> = ids
        .iter()
        .map(|&id| sim.world().log().changes_to_locus(id).next().map(|c| c.id))
        .collect();

    // Build stimuli: each entity gets a co-occurrence signal, with every other
    // entity in the document listed as an explicit predecessor.
    let stimuli: Vec<ProposedChange> = ids
        .iter()
        .enumerate()
        .map(|(i, &target_id)| {
            let predecessors: Vec<ChangeId> = last_change
                .iter()
                .enumerate()
                .filter_map(|(j, cid)| if j != i { *cid } else { None })
                .collect();
            ProposedChange::activation(target_id, COOCCUR, 1.0)
                .with_extra_predecessors(predecessors)
        })
        .collect();

    sim.step(stimuli);
    ids
}

// ── Main ──────────────────────────────────────────────────────────────────────

fn main() {
    let mut sim = SimulationBuilder::new()
        .locus_kind("ORG", EntityProgram)
        .locus_kind("PERSON", EntityProgram)
        .influence("cooccurrence", |cfg: InfluenceKindConfig| {
            cfg.with_decay(0.9)
                .with_stabilization(StabilizationConfig { alpha: 0.7 })
                .with_plasticity(PlasticityConfig {
                    learning_rate: 0.05,
                    weight_decay: 0.005,
                    max_weight: 5.0,

                    ..Default::default()
                })
        })
        .default_influence("cooccurrence")
        .build();

    // ── Ingest documents ──────────────────────────────────────────────────────

    println!("=== Ingesting documents ===");

    ingest_document(
        &mut sim,
        vec![
            (
                "Apple",
                "ORG",
                props! { "name" => "Apple",    "type" => "tech" },
            ),
            (
                "Tim_Cook",
                "PERSON",
                props! { "name" => "Tim_Cook", "role" => "CEO"  },
            ),
        ],
    );
    println!(
        "Doc 1: Apple + Tim_Cook  → {} relationships",
        sim.world().relationships().len()
    );

    ingest_document(
        &mut sim,
        vec![
            (
                "Elon_Musk",
                "PERSON",
                props! { "name" => "Elon_Musk", "role" => "CEO"  },
            ),
            (
                "Tesla",
                "ORG",
                props! { "name" => "Tesla",     "type" => "tech" },
            ),
        ],
    );
    println!(
        "Doc 2: Elon_Musk + Tesla → {} relationships",
        sim.world().relationships().len()
    );

    ingest_document(
        &mut sim,
        vec![
            (
                "Apple",
                "ORG",
                props! { "name" => "Apple",  "type" => "tech" },
            ),
            (
                "OpenAI",
                "ORG",
                props! { "name" => "OpenAI", "type" => "ai"   },
            ),
        ],
    );
    println!(
        "Doc 3: Apple + OpenAI    → {} relationships",
        sim.world().relationships().len()
    );

    // Bridge documents: connect the two clusters.
    ingest_document(
        &mut sim,
        vec![
            ("Tim_Cook", "PERSON", props! { "name" => "Tim_Cook" }),
            ("Tesla", "ORG", props! { "name" => "Tesla"    }),
        ],
    );
    println!(
        "Doc 4: Tim_Cook + Tesla  → {} relationships",
        sim.world().relationships().len()
    );

    ingest_document(
        &mut sim,
        vec![
            ("Elon_Musk", "PERSON", props! { "name" => "Elon_Musk" }),
            ("OpenAI", "ORG", props! { "name" => "OpenAI"    }),
        ],
    );
    println!(
        "Doc 5: Elon_Musk + OpenAI→ {} relationships",
        sim.world().relationships().len()
    );

    // ── Run a few more ticks to let Hebbian plasticity reinforce weights ──────
    println!("\n=== Reinforcement ticks ===");
    for doc in [
        vec![
            ("Apple", "ORG", props! { "name" => "Apple" }),
            ("Tim_Cook", "PERSON", props! { "name" => "Tim_Cook" }),
        ],
        vec![
            ("Elon_Musk", "PERSON", props! { "name" => "Elon_Musk" }),
            ("Tesla", "ORG", props! { "name" => "Tesla" }),
        ],
        vec![
            ("Apple", "ORG", props! { "name" => "Apple" }),
            ("OpenAI", "ORG", props! { "name" => "OpenAI" }),
        ],
    ] {
        ingest_document(&mut sim, doc);
    }

    // ── Query the emerged graph ────────────────────────────────────────────────
    let world_guard = sim.world();
    let world = &*world_guard;

    let name_of = |id| world.names().name_of(id).unwrap_or("?");

    println!("\n=== Emerged relationships ===");
    let mut rels: Vec<_> = world.relationships().iter().collect();
    rels.sort_by(|a, b| {
        let wa = a.state.as_slice().get(1).copied().unwrap_or(0.0);
        let wb = b.state.as_slice().get(1).copied().unwrap_or(0.0);
        wb.total_cmp(&wa)
    });
    for rel in &rels {
        let (a, b) = match rel.endpoints {
            graph_core::Endpoints::Directed { from, to } => (from, to),
            graph_core::Endpoints::Symmetric { a, b } => (a, b),
        };
        let activity = rel.state.as_slice().first().copied().unwrap_or(0.0);
        let weight = rel.state.as_slice().get(1).copied().unwrap_or(0.0);
        println!(
            "  {} ↔ {}  activity={activity:.3}  weight={weight:.3}",
            name_of(a),
            name_of(b)
        );
    }

    println!("\n=== Connected components ===");
    let components = connected_components(world);
    for (i, comp) in components.iter().enumerate() {
        let mut names: Vec<_> = comp.iter().map(|&id| name_of(id)).collect();
        names.sort_unstable();
        println!("  component {}: {:?}", i + 1, names);
    }

    let apple_id = sim.resolve("Apple").unwrap();
    let elon_id = sim.resolve("Elon_Musk").unwrap();

    println!("\n=== Path: Apple → Elon_Musk ===");
    match path_between(world, apple_id, elon_id) {
        Some(path) => {
            let names: Vec<_> = path.iter().map(|&id| name_of(id)).collect();
            println!("  {:?}", names);
        }
        None => println!("  (no path found)"),
    }

    println!("\n=== Reachable from Apple within 2 hops ===");
    let reachable = reachable_from(world, apple_id, 2);
    let mut names: Vec<_> = reachable
        .iter()
        .filter(|&&id| id != apple_id)
        .map(|&id| name_of(id))
        .collect();
    names.sort_unstable();
    println!("  {:?}", names);

    println!("\n=== Final locus activations ===");
    let mut loci: Vec<_> = world.loci().iter().collect();
    loci.sort_by_key(|l| l.id.0);
    for locus in loci {
        let activation = locus.state.as_slice().first().copied().unwrap_or(0.0);
        let entity_type = world
            .properties()
            .get(locus.id)
            .and_then(|p| p.get_str("type").or_else(|| p.get_str("role")))
            .unwrap_or("?");
        println!(
            "  [{}] ({entity_type})  activation={activation:.3}",
            name_of(locus.id)
        );
    }

    println!(
        "\nDone. {} entities, {} co-occurrence relationships emerged.",
        world.loci().len(),
        world.relationships().len(),
    );
}
