//! Minimal example: two loci, one influence kind, two ticks.
//!
//! This is the smallest runnable simulation:
//!
//! 1. Register one locus kind and one influence kind.
//! 2. Call `ingest_cooccurrence` twice with the same two nodes — the first
//!    call creates the loci; the second call wires cross-locus predecessors
//!    and triggers the engine to auto-emerge a relationship between them.
//! 3. Print the emerged relationship.
//!
//! Run: `cargo run -p graph-engine --example minimal`

use graph_core::{Change, Locus, LocusContext, LocusProgram, ProposedChange, props};
use graph_engine::{InfluenceKindConfig, SimulationBuilder};

/// A do-nothing program. Receives stimuli; produces no follow-up changes.
struct NoopProgram;

impl LocusProgram for NoopProgram {
    fn process(&self, _: &Locus, _: &[&Change], _: &dyn LocusContext) -> Vec<ProposedChange> {
        vec![]
    }
}

fn main() {
    // ── Build ──────────────────────────────────────────────────────────────────

    let mut sim = SimulationBuilder::new()
        .locus_kind("NODE", NoopProgram)
        .influence("signal", |cfg: InfluenceKindConfig| cfg.with_decay(0.9).symmetric())
        .default_influence("signal")
        .build();

    // ── Ingest ─────────────────────────────────────────────────────────────────

    // First call: creates the loci. No relationship yet (no prior changes
    // to wire as cross-locus predecessors).
    sim.ingest_cooccurrence(vec![
        ("alice", "NODE", props! { "name" => "alice" }),
        ("bob",   "NODE", props! { "name" => "bob"   }),
    ]);
    println!("after 1st co-occurrence: {} relationships", sim.world().relationships().len());

    // Second call: loci already exist → cross-locus predecessors fire →
    // the engine auto-emerges a relationship between alice and bob.
    sim.ingest_cooccurrence(vec![
        ("alice", "NODE", props! { "name" => "alice" }),
        ("bob",   "NODE", props! { "name" => "bob"   }),
    ]);
    println!("after 2nd co-occurrence: {} relationships", sim.world().relationships().len());

    // ── Inspect ────────────────────────────────────────────────────────────────

    let world_guard = sim.world();
    let world = &*world_guard;
    for rel in world.relationships().iter() {
        let (a, b) = match rel.endpoints {
            graph_core::Endpoints::Symmetric { a, b } => (a, b),
            graph_core::Endpoints::Directed { from, to } => (from, to),
        };
        let name_a = world.names().name_of(a).unwrap_or("?");
        let name_b = world.names().name_of(b).unwrap_or("?");
        println!("  {} ↔ {}  activity={:.3}", name_a, name_b, rel.activity());
    }
}
