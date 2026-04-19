//! Multi-kind relationship profile example.
//!
//! Demonstrates that `profile_similarity` produces **non-trivial** values when
//! locus pairs are coupled through different mixes of influence kinds.  Every
//! existing single-kind example produces collinear kind-vectors, so all pairwise
//! profile similarities collapse to 1.0.  This example breaks that degeneracy.
//!
//! ## Domain: research collaboration network
//!
//! Five researchers interact through three distinct influence kinds:
//!
//! | Kind            | Meaning                                    | Valence  |
//! |-----------------|---------------------------------------------|----------|
//! | `TRUST_KIND`    | interpersonal trust / endorsement           | positive |
//! | `COLLAB_KIND`   | active collaboration / co-authorship signal | positive |
//! | `CHALLENGE_KIND`| intellectual challenge / debate             | negative |
//!
//! Topology and intended coupling profiles:
//!
//! ```text
//!   ALICE ──trust──→ BOB     (ALICE↔BOB:  trust + collab)
//!   ALICE ──trust──→ CAROL   (ALICE↔CAROL: trust only)
//!   BOB   ──collab─→ ALICE
//!   BOB   ──collab─→ DAVE    (BOB↔DAVE:   collab + challenge)
//!   BOB   ──challenge→ DAVE
//!   CAROL ──trust──→ ALICE
//!   CAROL ──challenge→ EVE   (CAROL↔EVE:  challenge only)
//!   DAVE  ──collab─→ BOB
//!   EVE   ──challenge→ CAROL
//! ```
//!
//! After several ticks the engine auto-emerges one relationship per
//! `(locus-pair, kind)`.  The four distinct pairs then have genuinely
//! different kind-indexed activity vectors, which makes `profile_similarity`
//! interesting:
//!
//! | Pair comparison               | Expected similarity |
//! |-------------------------------|---------------------|
//! | ALICE↔BOB  vs ALICE↔CAROL     | ~0.7  (share trust; AB also has collab) |
//! | ALICE↔BOB  vs BOB↔DAVE        | ~0.4  (share collab; AB has trust, BD has challenge) |
//! | ALICE↔BOB  vs CAROL↔EVE       | ~0.0  (orthogonal: trust+collab vs challenge) |
//! | CAROL↔EVE  vs BOB↔DAVE        | ~0.6  (both have challenge; BD also has collab) |
//!
//! The example also demonstrates:
//! - `net_activity_with_interactions`: trust+collab synergy (boost=1.3),
//!   trust+challenge antagonism (dampen=0.7).
//! - `infer_transitive`: composed trust chain ALICE → BOB → DAVE (COLLAB kind).
//! - `most_similar_relationships`: which individual edge is most like
//!   the ALICE→BOB trust edge.
//!
//! Run: `cargo run -p graph-engine --example multi_kind_profile`

use graph_core::{
    Change, Endpoints, InfluenceKindId, InteractionEffect, Locus, LocusContext, LocusId,
    LocusKindId, LocusProgram, ProposedChange, StateVector,
};
use graph_engine::{
    DefaultCoherePerspective, DefaultEmergencePerspective, Engine, EngineConfig,
    InfluenceKindConfig, InfluenceKindRegistry, LocusKindRegistry,
};
use graph_query as Q;
use graph_world::World;

// ── Kind constants ─────────────────────────────────────────────────────────────

const KIND_RESEARCHER: LocusKindId = LocusKindId(1);

/// Interpersonal trust / endorsement.
const TRUST_KIND: InfluenceKindId = InfluenceKindId(1);
/// Active collaboration / co-authorship signal.
const COLLAB_KIND: InfluenceKindId = InfluenceKindId(2);
/// Intellectual challenge / debate.
const CHALLENGE_KIND: InfluenceKindId = InfluenceKindId(3);

// ── Locus IDs ──────────────────────────────────────────────────────────────────

const ALICE: LocusId = LocusId(1);
const BOB: LocusId = LocusId(2);
const CAROL: LocusId = LocusId(3);
const DAVE: LocusId = LocusId(4);
const EVE: LocusId = LocusId(5);

fn label(id: LocusId) -> &'static str {
    match id {
        ALICE => "ALICE",
        BOB => "BOB",
        CAROL => "CAROL",
        DAVE => "DAVE",
        EVE => "EVE",
        _ => "?",
    }
}

// ── Programs ───────────────────────────────────────────────────────────────────

/// Generic researcher program.
///
/// When a change of kind K arrives, forward a proportional signal of the same
/// kind to each target declared for that kind.  This encodes the topology
/// (who trusts / collaborates with / challenges whom) entirely in the program
/// configuration, keeping the engine mechanics generic.
///
/// The forwarded magnitude is `incoming_sum * attenuation`.  Signals smaller
/// than `min_signal` are dropped to prevent tiny magnitudes from cycling
/// indefinitely when the graph contains back-edges (e.g. CAROL↔EVE challenge).
struct ResearcherProgram {
    trust_targets: Vec<LocusId>,
    collab_targets: Vec<LocusId>,
    challenge_targets: Vec<LocusId>,
    /// Signal attenuation per forwarding step.
    attenuation: f32,
    /// Minimum signal magnitude to forward.  Drops the change if below this.
    min_signal: f32,
}

impl LocusProgram for ResearcherProgram {
    fn process(
        &self,
        _locus: &Locus,
        incoming: &[&Change],
        _ctx: &dyn LocusContext,
    ) -> Vec<ProposedChange> {
        if incoming.is_empty() {
            return vec![];
        }

        let mut out = Vec::new();

        // Sum incoming magnitudes per kind; forward attenuated signal.
        let sum_kind = |kind: InfluenceKindId| -> f32 {
            incoming
                .iter()
                .filter(|c| c.kind == kind)
                .map(|c| c.after.as_slice().first().copied().unwrap_or(0.0).abs())
                .sum::<f32>()
        };

        let trust_in = sum_kind(TRUST_KIND);
        if trust_in >= self.min_signal {
            let mag = trust_in * self.attenuation;
            for &target in &self.trust_targets {
                out.push(ProposedChange::stimulus(target, TRUST_KIND, &[mag]));
            }
        }

        let collab_in = sum_kind(COLLAB_KIND);
        if collab_in >= self.min_signal {
            let mag = collab_in * self.attenuation;
            for &target in &self.collab_targets {
                out.push(ProposedChange::stimulus(target, COLLAB_KIND, &[mag]));
            }
        }

        let challenge_in = sum_kind(CHALLENGE_KIND);
        if challenge_in >= self.min_signal {
            let mag = challenge_in * self.attenuation;
            for &target in &self.challenge_targets {
                out.push(ProposedChange::stimulus(target, CHALLENGE_KIND, &[-mag]));
            }
        }

        out
    }
}

// ── World construction ─────────────────────────────────────────────────────────

fn build_world() -> (World, LocusKindRegistry, InfluenceKindRegistry) {
    let mut loci = LocusKindRegistry::new();

    // ALICE: trusts BOB and CAROL; collaborates with BOB.
    loci.insert(
        KIND_RESEARCHER,
        Box::new(ResearcherProgram {
            trust_targets: vec![BOB, CAROL],
            collab_targets: vec![BOB],
            challenge_targets: vec![],
            attenuation: 0.5,
            min_signal: 0.05,
        }),
    );

    // Individual programs are needed because each researcher has different
    // routing.  We use separate LocusKindId values to keep programs independent.
    //
    // For conciseness, BOB, CAROL, DAVE, EVE use LocusKindId(2..5) but share
    // the same ResearcherProgram struct with different target lists.

    const KIND_BOB: LocusKindId = LocusKindId(2);
    const KIND_CAROL: LocusKindId = LocusKindId(3);
    const KIND_DAVE: LocusKindId = LocusKindId(4);
    const KIND_EVE: LocusKindId = LocusKindId(5);

    // BOB: trusts ALICE; collaborates with ALICE and DAVE; challenges DAVE.
    loci.insert(
        KIND_BOB,
        Box::new(ResearcherProgram {
            trust_targets: vec![ALICE],
            collab_targets: vec![ALICE, DAVE],
            challenge_targets: vec![DAVE],
            attenuation: 0.5,
            min_signal: 0.05,
        }),
    );

    // CAROL: trusts ALICE; challenges EVE.
    loci.insert(
        KIND_CAROL,
        Box::new(ResearcherProgram {
            trust_targets: vec![ALICE],
            collab_targets: vec![],
            challenge_targets: vec![EVE],
            attenuation: 0.5,
            min_signal: 0.05,
        }),
    );

    // DAVE: collaborates with BOB only.
    loci.insert(
        KIND_DAVE,
        Box::new(ResearcherProgram {
            trust_targets: vec![],
            collab_targets: vec![BOB],
            challenge_targets: vec![],
            attenuation: 0.5,
            min_signal: 0.05,
        }),
    );

    // EVE: challenges CAROL only.
    loci.insert(
        KIND_EVE,
        Box::new(ResearcherProgram {
            trust_targets: vec![],
            collab_targets: vec![],
            challenge_targets: vec![CAROL],
            attenuation: 0.5,
            min_signal: 0.05,
        }),
    );

    let mut influences = InfluenceKindRegistry::new();
    influences.insert(
        TRUST_KIND,
        InfluenceKindConfig::new("trust").with_decay(0.85),
    );
    influences.insert(
        COLLAB_KIND,
        InfluenceKindConfig::new("collab").with_decay(0.85),
    );
    influences.insert(
        CHALLENGE_KIND,
        InfluenceKindConfig::new("challenge").with_decay(0.85),
    );

    // Cross-kind interaction rules used later by `net_activity_with_interactions`.
    // Trust + collab together are synergistic (boost 1.3×).
    influences.register_interaction(
        TRUST_KIND,
        COLLAB_KIND,
        InteractionEffect::Synergistic { boost: 1.3 },
    );
    // Trust and challenge partially cancel (dampen 0.7×).
    influences.register_interaction(
        TRUST_KIND,
        CHALLENGE_KIND,
        InteractionEffect::Antagonistic { dampen: 0.7 },
    );

    let mut world = World::new();
    world.insert_locus(Locus::new(ALICE, KIND_RESEARCHER, StateVector::zeros(1)));
    world.insert_locus(Locus::new(BOB, KIND_BOB, StateVector::zeros(1)));
    world.insert_locus(Locus::new(CAROL, KIND_CAROL, StateVector::zeros(1)));
    world.insert_locus(Locus::new(DAVE, KIND_DAVE, StateVector::zeros(1)));
    world.insert_locus(Locus::new(EVE, KIND_EVE, StateVector::zeros(1)));

    (world, loci, influences)
}

// ── Helpers ────────────────────────────────────────────────────────────────────

fn print_bundle(label_a: &str, label_b: &str, bundle: &Q::RelationshipBundle<'_>) {
    let dominant = bundle.dominant_kind().map(kind_name).unwrap_or("-");
    println!(
        "  {}↔{}  edges={}  net_activity={:.3}  dominant={}",
        label_a,
        label_b,
        bundle.len(),
        bundle.net_activity(),
        dominant,
    );
    for (kind, act) in bundle.activity_by_kind() {
        println!("    {:<9}  activity={act:+.3}", kind_name(kind));
    }
}

fn kind_name(id: InfluenceKindId) -> &'static str {
    match id {
        TRUST_KIND => "trust",
        COLLAB_KIND => "collab",
        CHALLENGE_KIND => "challenge",
        _ => "?",
    }
}

// ── Main ───────────────────────────────────────────────────────────────────────

fn main() {
    let (mut world, loci, influences) = build_world();
    // max_batches_per_tick=64: cyclic topologies (e.g. CAROL↔EVE challenge)
    // require more batches to quiesce.  With attenuation=0.5 and min_signal=0.05,
    // the longest chain dies within ~10 batches; 64 is a comfortable ceiling.
    let engine = Engine::new(EngineConfig {
        max_batches_per_tick: 64,
    });

    println!("=== Multi-Kind Relationship Profile Example ===\n");
    println!("  Three influence kinds: TRUST (+), COLLAB (+), CHALLENGE (-)");
    println!("  Pairs: ALICE↔BOB=trust+collab  ALICE↔CAROL=trust");
    println!("         BOB↔DAVE=collab+challenge  CAROL↔EVE=challenge\n");

    // ── Tick 1: prime every relationship type ─────────────────────────────────
    //
    // Inject one stimulus of each kind into the node that originates it.
    // The programs forward the signal to their targets, creating cross-locus
    // causal flow which the engine uses to auto-emerge directed relationships.

    println!("--- Tick 1: initial stimuli (all three kinds) ---");
    let r1 = engine.tick(
        &mut world,
        &loci,
        &influences,
        vec![
            // Trust stimuli: Alice initiates trust → BOB, CAROL
            ProposedChange::stimulus(ALICE, TRUST_KIND, &[1.0]),
            // Collab stimuli: Bob initiates collaboration → ALICE, DAVE
            ProposedChange::stimulus(BOB, COLLAB_KIND, &[1.0]),
            // Challenge: Carol challenges EVE; Bob challenges DAVE
            ProposedChange::stimulus(CAROL, CHALLENGE_KIND, &[1.0]),
            ProposedChange::stimulus(BOB, CHALLENGE_KIND, &[0.6]),
        ],
    );
    println!(
        "  batches={} changes={} relationships={}",
        r1.batches_committed,
        r1.changes_committed,
        world.relationships().len()
    );

    // ── Tick 2: reinforce — each node now relays what it received ─────────────

    println!("\n--- Tick 2: reinforcement ---");
    let r2 = engine.tick(
        &mut world,
        &loci,
        &influences,
        vec![
            ProposedChange::stimulus(ALICE, TRUST_KIND, &[1.0]),
            ProposedChange::stimulus(BOB, COLLAB_KIND, &[1.0]),
            ProposedChange::stimulus(BOB, TRUST_KIND, &[0.8]),
            ProposedChange::stimulus(CAROL, CHALLENGE_KIND, &[1.0]),
            ProposedChange::stimulus(EVE, CHALLENGE_KIND, &[0.9]),
            ProposedChange::stimulus(DAVE, COLLAB_KIND, &[0.7]),
        ],
    );
    println!(
        "  batches={} changes={} relationships={}",
        r2.batches_committed,
        r2.changes_committed,
        world.relationships().len()
    );

    // ── Tick 3: second reinforcement — let activity accumulate ─────────────────

    println!("\n--- Tick 3: second reinforcement ---");
    let r3 = engine.tick(
        &mut world,
        &loci,
        &influences,
        vec![
            ProposedChange::stimulus(ALICE, TRUST_KIND, &[1.0]),
            ProposedChange::stimulus(BOB, COLLAB_KIND, &[1.0]),
            ProposedChange::stimulus(BOB, TRUST_KIND, &[0.8]),
            ProposedChange::stimulus(CAROL, CHALLENGE_KIND, &[1.0]),
            ProposedChange::stimulus(EVE, CHALLENGE_KIND, &[0.9]),
            ProposedChange::stimulus(DAVE, COLLAB_KIND, &[0.7]),
        ],
    );
    println!(
        "  batches={} changes={} relationships={}",
        r3.batches_committed,
        r3.changes_committed,
        world.relationships().len()
    );

    // ── Relationship inventory ─────────────────────────────────────────────────

    println!("\n--- Emerged relationships ---");
    let mut rels: Vec<_> = world.relationships().iter().collect();
    rels.sort_by_key(|r| r.id);
    for r in &rels {
        let (a, b) = match r.endpoints {
            Endpoints::Directed { from, to } => (from, to),
            Endpoints::Symmetric { a, b } => (a, b),
        };
        println!(
            "  rel#{:2}  {}→{}  kind={:<9}  activity={:+.3}  weight={:.4}",
            r.id.0,
            label(a),
            label(b),
            kind_name(r.kind),
            r.activity(),
            r.weight()
        );
    }

    // ── Relationship profiles (multi-dimensional view) ─────────────────────────

    println!("\n--- Relationship profiles (all kinds, both directions) ---");
    let bundle_ab = Q::relationship_profile(&world, ALICE, BOB);
    let bundle_ac = Q::relationship_profile(&world, ALICE, CAROL);
    let bundle_bd = Q::relationship_profile(&world, BOB, DAVE);
    let bundle_ce = Q::relationship_profile(&world, CAROL, EVE);

    print_bundle("ALICE", "BOB", &bundle_ab);
    print_bundle("ALICE", "CAROL", &bundle_ac);
    print_bundle("BOB", "DAVE", &bundle_bd);
    print_bundle("CAROL", "EVE", &bundle_ce);

    // ── Profile similarity matrix ─────────────────────────────────────────────
    //
    // This is the key output.  With a single kind all similarities would be 1.0.
    // The multi-kind topology gives genuine angular separation between profiles.

    println!("\n--- Profile similarity matrix (cosine on kind-indexed activity vectors) ---");
    println!("  (1.0 = identical coupling profile; 0.0 = orthogonal; −1.0 = opposite)");

    let pairs = [
        ("ALICE↔BOB", &bundle_ab),
        ("ALICE↔CAROL", &bundle_ac),
        ("BOB↔DAVE", &bundle_bd),
        ("CAROL↔EVE", &bundle_ce),
    ];

    // Header row
    print!("  {:13}", "");
    for (name, _) in &pairs {
        print!("  {:13}", name);
    }
    println!();

    // Similarity matrix
    for (name_i, b_i) in &pairs {
        print!("  {:13}", name_i);
        for (_, b_j) in &pairs {
            let sim = b_i.profile_similarity(b_j);
            print!("  {:+.3}         ", sim);
        }
        println!();
    }

    // ── Net activity with cross-kind interactions ──────────────────────────────
    //
    // `net_activity_with_interactions` applies the synergy/antagonism rules
    // registered on the InfluenceKindRegistry.

    println!("\n--- Net activity with cross-kind interactions ---");
    println!("  Rules: trust+collab → synergistic (×1.3); trust+challenge → antagonistic (×0.7)");

    let interaction_fn =
        |ka: InfluenceKindId, kb: InfluenceKindId| influences.interaction_between(ka, kb).cloned();

    for (name, bundle) in &pairs {
        let plain = bundle.net_activity();
        let adj = bundle.net_activity_with_interactions(&interaction_fn);
        println!("  {name:13}  plain={plain:+.3}  adjusted={adj:+.3}");
    }

    // ── Transitive inference: collab chain ALICE → BOB → DAVE ─────────────────
    //
    // `infer_transitive` composes activities along the shortest directed path
    // of `kind`.  Product rule: composed = edge1 × edge2 × … (attenuates with hops).
    // Min rule:     composed = min(edge1, edge2, …) (bottleneck strength).

    println!("\n--- Transitive inference: COLLAB chain ALICE → DAVE (through BOB) ---");
    let t_product =
        Q::infer_transitive(&world, ALICE, DAVE, COLLAB_KIND, Q::TransitiveRule::Product);
    let t_min = Q::infer_transitive(&world, ALICE, DAVE, COLLAB_KIND, Q::TransitiveRule::Min);
    let t_mean = Q::infer_transitive(&world, ALICE, DAVE, COLLAB_KIND, Q::TransitiveRule::Mean);

    // Product rule multiplies raw activity values along the path.  When those
    // values are > 1.0 the product grows with distance — suited for probability
    // chains only when activities have been normalised to [0, 1].  Min and Mean
    // rules are more robust for unnormalised activity.
    println!(
        "  ALICE→DAVE (Product): {}",
        t_product
            .map(|v| format!("{v:.4}"))
            .unwrap_or("no collab path".into())
    );
    println!(
        "  ALICE→DAVE (Min):     {}",
        t_min
            .map(|v| format!("{v:.4}"))
            .unwrap_or("no collab path".into())
    );
    println!(
        "  ALICE→DAVE (Mean):    {}",
        t_mean
            .map(|v| format!("{v:.4}"))
            .unwrap_or("no collab path".into())
    );

    // Also check TRUST chain from ALICE to CAROL (direct hop).
    let t_trust_direct =
        Q::infer_transitive(&world, ALICE, CAROL, TRUST_KIND, Q::TransitiveRule::Product);
    println!(
        "  ALICE→CAROL (Trust, Product, 1 hop): {}",
        t_trust_direct
            .map(|v| format!("{v:.4}"))
            .unwrap_or("no trust path".into())
    );

    // ── Most similar relationships to the ALICE→BOB trust edge ────────────────
    //
    // `most_similar_relationships` uses cosine similarity on the raw StateVectors
    // (not the kind-indexed profile vectors from RelationshipBundle). This ranks
    // individual edges rather than bundles, and uses the 2-slot [activity, weight]
    // geometry.
    //
    // Note: in this example all weights remain 0.0 (Hebbian plasticity is off and
    // no extra_slots are populated), so every edge has a StateVector of the form
    // [activity, 0.0].  All positive-activity edges are collinear in that 2D space
    // → cosine similarity collapses to 1.0 for every comparison.
    //
    // To get non-trivial scores from `most_similar_relationships`, enable Hebbian
    // plasticity (PlasticityConfig) or add user-populated extra_slots so that the
    // [activity, weight, ...] vectors genuinely differ across edges.

    println!("\n--- Most similar edges to the ALICE→BOB TRUST edge ---");
    println!("  (all similarities are 1.0 here: weights=0 makes every active edge collinear)");
    // Find the ALICE→BOB trust relationship ID.
    let alice_bob_trust_id = world
        .relationships_between(ALICE, BOB)
        .find(|r| r.kind == TRUST_KIND)
        .map(|r| r.id);

    match alice_bob_trust_id {
        Some(rid) => {
            let similar = Q::most_similar_relationships(&world, rid, 4);
            for (other_id, sim) in &similar {
                let rel = world.relationships().get(*other_id).unwrap();
                let (a, b) = match rel.endpoints {
                    Endpoints::Directed { from, to } => (from, to),
                    Endpoints::Symmetric { a, b } => (a, b),
                };
                println!(
                    "  rel#{:2}  {}→{}  kind={:<9}  cosine_sim={:.4}",
                    other_id.0,
                    label(a),
                    label(b),
                    kind_name(rel.kind),
                    sim
                );
            }
        }
        None => println!("  ALICE→BOB TRUST edge not yet emerged"),
    }

    // ── Entity emergence ───────────────────────────────────────────────────────

    let ep = DefaultEmergencePerspective::default();
    engine.recognize_entities(&mut world, &influences, &ep);

    println!(
        "\n--- Entities ({} active) ---",
        world.entities().active_count()
    );
    for e in world.entities().active() {
        let members: Vec<&str> = e.current.members.iter().map(|l| label(*l)).collect();
        println!(
            "  entity#{}  members=[{}]  coherence={:.3}",
            e.id.0,
            members.join(", "),
            e.current.coherence,
        );
    }

    // ── Cohere clusters ────────────────────────────────────────────────────────

    let cp = DefaultCoherePerspective::default();
    engine.extract_cohere(&mut world, &influences, &cp);
    let coheres = world.coheres().get("default").unwrap_or(&[]);
    println!("\n--- Coheres ({}) ---", coheres.len());
    for c in coheres {
        let ms = match &c.members {
            graph_core::CohereMembers::Entities(ids) => ids
                .iter()
                .map(|e| format!("entity#{}", e.0))
                .collect::<Vec<_>>()
                .join(", "),
            _ => "(mixed)".into(),
        };
        println!("  cohere#{}  [{}]  strength={:.3}", c.id.0, ms, c.strength);
    }
    if coheres.is_empty() {
        println!("  (none — try lowering min_bridge_activity)");
    }

    println!("\nDone.");
}
