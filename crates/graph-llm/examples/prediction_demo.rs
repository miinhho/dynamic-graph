//! Dynamic graph engine: future-prediction test suite.
//!
//! Three prediction tasks on Zachary's Karate Club (1977) — a classic
//! 34-node / 78-edge social network with a known ground-truth faction split:
//!
//!  1. **Faction split prediction** — can Hebbian dynamics discover the
//!     two-community partition without ever being told the labels?
//!  2. **Link prediction** — given a graph with 10 edges removed, can the
//!     engine predict which pairs are "missing"?
//!  3. **Weight trend forecasting** — do intra-faction edges trend Rising
//!     while cross-faction bridges trend Falling or Stable?
//!     (Now backed by ChangeLog entries so `relationship_weight_trend` works.)
//!
//! Each task is scored by an LLM which provides a qualitative evaluation.
//!
//! Run with:
//!   cargo run -p graph-llm --example prediction_demo --features ollama

#[cfg(not(feature = "ollama"))]
fn main() {
    eprintln!("Run with: cargo run -p graph-llm --example prediction_demo --features ollama");
}

#[cfg(feature = "ollama")]
fn main() {
    use graph_llm::OllamaClient;

    let model = std::env::var("OLLAMA_MODEL").unwrap_or_else(|_| "llama3:8b".to_owned());
    let client = OllamaClient::new(&model);

    println!("=== Dynamic Graph Engine: Prediction Test Suite ===");
    println!("    Dataset : Zachary's Karate Club (1977) — 34 nodes, 78 edges");
    println!("    LLM     : {model}");
    println!();

    task1_faction_split(&client);
    task2_link_prediction(&client);
    task3_trend_forecasting(&client);

    println!("=== done ===");
}

// ─── Ground truth ─────────────────────────────────────────────────────────────

/// Mr. Hi's faction: 16 members who sided with node 0.
const MR_HI: &[u64] = &[0, 1, 2, 3, 4, 5, 6, 7, 8, 10, 11, 12, 13, 17, 19, 21];
/// Administrator's faction: 18 members who sided with node 33.
const OFFICER: &[u64] = &[9, 14, 15, 16, 18, 20, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33];

/// All 78 undirected edges (smaller id first).
const ALL_EDGES: &[(u64, u64)] = &[
    (0,1),(0,2),(0,3),(0,4),(0,5),(0,6),(0,7),(0,8),(0,10),(0,11),
    (0,12),(0,13),(0,17),(0,19),(0,21),(0,31),
    (1,2),(1,3),(1,7),(1,13),(1,17),(1,19),(1,21),(1,30),
    (2,3),(2,7),(2,8),(2,9),(2,13),(2,27),(2,28),(2,32),
    (3,7),(3,12),(3,13),
    (4,6),(4,10),
    (5,6),(5,10),(5,16),
    (6,16),
    (8,30),(8,32),(8,33),
    (9,33),
    (13,33),
    (14,32),(14,33),
    (15,32),(15,33),
    (18,32),(18,33),
    (19,33),
    (20,32),(20,33),
    (22,32),(22,33),
    (23,25),(23,27),(23,29),(23,32),(23,33),
    (24,25),(24,27),(24,31),
    (25,31),
    (26,29),(26,33),
    (27,33),
    (28,31),(28,33),
    (29,32),(29,33),
    (30,32),(30,33),
    (31,32),(31,33),
    (32,33),
];

/// Edges held out for link prediction — 5 intra-Hi + 5 intra-Officer,
/// none touching either hub (node 0 or node 33).
const HOLDOUT_EDGES: &[(u64, u64)] = &[
    // Intra-Hi
    (1, 2), (1, 3), (4, 6), (4, 10), (5, 6),
    // Intra-Officer
    (23, 25), (23, 27), (24, 25), (24, 27), (26, 29),
];

/// True-negative pairs: known non-edges, one endpoint per faction.
const TRUE_NEGATIVES: &[(u64, u64)] = &[
    (1, 9), (2, 14), (3, 15), (7, 22), (4, 23),
];

// ─── Shared gossip program ────────────────────────────────────────────────────

/// Propagates 5% of incoming signal to all symmetric neighbors.
/// Quiesces within ~2 hops (0.05^2 = 0.0025 < threshold 0.001).
#[cfg(feature = "ollama")]
struct SpreadProgram;

#[cfg(feature = "ollama")]
impl graph_core::LocusProgram for SpreadProgram {
    fn process(
        &self,
        locus: &graph_core::Locus,
        incoming: &[&graph_core::Change],
        ctx: &dyn graph_core::LocusContext,
    ) -> Vec<graph_core::ProposedChange> {
        use graph_core::{ChangeSubject, Endpoints, StateVector};
        const SOCIAL: graph_core::InfluenceKindId = graph_core::InfluenceKindId(1);

        let signal: f32 = incoming.iter().filter_map(|c| match c.subject {
            ChangeSubject::Locus(_) => c.after.as_slice().first().copied(),
            _ => None,
        }).sum();
        if signal.abs() < 0.001 { return Vec::new(); }

        ctx.relationships_for(locus.id).filter_map(|rel| {
            let neighbor = match rel.endpoints {
                Endpoints::Symmetric { a, b } => if a == locus.id { b } else { a },
                Endpoints::Directed { from, to } => if from == locus.id { to } else { return None; },
            };
            Some(graph_core::ProposedChange::new(
                ChangeSubject::Locus(neighbor),
                SOCIAL,
                StateVector::from_slice(&[signal * 0.05]),
            ))
        }).collect()
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

#[cfg(feature = "ollama")]
fn neighbors_of_in(node: u64, edges: &[(u64, u64)]) -> Vec<u64> {
    edges.iter().filter_map(|&(a, b)| {
        if a == node { Some(b) } else if b == node { Some(a) } else { None }
    }).collect()
}

/// Adamic-Adar activity weight for edge (u, v) within a given edge set.
#[cfg(feature = "ollama")]
fn edge_activity(u: u64, v: u64, edges: &[(u64, u64)]) -> f32 {
    use rustc_hash::FxHashSet;
    let nu: FxHashSet<u64> = neighbors_of_in(u, edges).into_iter().collect();
    let nv: FxHashSet<u64> = neighbors_of_in(v, edges).into_iter().collect();
    let aa: f32 = nu.intersection(&nv).map(|&w| {
        let deg = neighbors_of_in(w, edges).len().max(2);
        1.0 / (deg as f32).ln()
    }).sum();
    1.0 + aa
}

#[cfg(feature = "ollama")]
fn is_hi(node: u64)      -> bool { MR_HI.contains(&node) }
#[cfg(feature = "ollama")]
fn is_officer(node: u64) -> bool { OFFICER.contains(&node) }

#[cfg(feature = "ollama")]
fn indent(text: &str, prefix: &str) -> String {
    text.lines().map(|l| format!("{prefix}{l}")).collect::<Vec<_>>().join("\n")
}

// ─── Task 1: Faction split prediction ────────────────────────────────────────

#[cfg(feature = "ollama")]
fn task1_faction_split(client: &graph_llm::OllamaClient) {
    use graph_llm::score_prediction;
    use graph_core::{
        BatchId, ChangeSubject, Endpoints, InfluenceKindId, Locus, LocusId, LocusKindId,
        ProposedChange, Relationship, RelationshipLineage, StateVector,
    };
    use graph_engine::{
        DefaultEmergencePerspective, Engine, EngineConfig, InfluenceKindConfig,
        InfluenceKindRegistry, LocusKindRegistry, PlasticityConfig,
    };
    use graph_world::World;

    println!("━━━ Task 1: Faction split prediction ━━━");
    println!("  Setup  : 78 edges with Adamic-Adar weights");
    println!("  Method : 8 ticks of gossip + Hebbian plasticity → recognize_entities");
    println!("  Oracle : MR_HI (16 nodes) vs OFFICER (18 nodes)\n");

    const SOCIAL: InfluenceKindId = InfluenceKindId(1);
    const MEMBER: LocusKindId     = LocusKindId(1);

    let mut world = World::new();
    for i in 0..34u64 {
        world.insert_locus(Locus::new(LocusId(i), MEMBER, StateVector::zeros(1)));
    }
    for &(a, b) in ALL_EDGES {
        let id  = world.relationships_mut().mint_id();
        let act = edge_activity(a, b, ALL_EDGES);
        world.relationships_mut().insert(Relationship {
            id, kind: SOCIAL,
            endpoints: Endpoints::Symmetric { a: LocusId(a), b: LocusId(b) },
            state: StateVector::from_slice(&[act, 0.0]),
            lineage: RelationshipLineage::new_synthetic(SOCIAL),
            created_batch: BatchId(0), last_decayed_batch: 0, metadata: None,
        });
    }

    let mut loci_reg = LocusKindRegistry::new();
    loci_reg.insert(MEMBER, Box::new(SpreadProgram));
    let mut inf_reg = InfluenceKindRegistry::new();
    inf_reg.insert(SOCIAL, InfluenceKindConfig::new("social_tie")
        .with_symmetric(true)
        .with_decay(0.98)
        .with_plasticity(PlasticityConfig { learning_rate: 0.1, max_weight: 10.0, weight_decay: 0.99, stdp: false,
            ..Default::default() }));

    let engine = Engine::new(EngineConfig { max_batches_per_tick: 3 });

    for _ in 0..8 {
        let stimuli = vec![
            ProposedChange::new(ChangeSubject::Locus(LocusId(0)),  SOCIAL, StateVector::from_slice(&[1.0])),
            ProposedChange::new(ChangeSubject::Locus(LocusId(33)), SOCIAL, StateVector::from_slice(&[1.0])),
        ];
        engine.tick(&mut world, &loci_reg, &inf_reg, stimuli);
    }

    let perspective = DefaultEmergencePerspective::default();
    engine.recognize_entities(&mut world, &inf_reg, &perspective);

    let entities: Vec<_> = world.entities().active().collect();
    let n_entities = entities.len();

    let hi_entity  = entities.iter().find(|e| e.current.members.contains(&LocusId(0)));
    let off_entity = entities.iter().find(|e| e.current.members.contains(&LocusId(33)));

    let (hi_correct, hi_total) = hi_entity.map(|e| {
        let correct = e.current.members.iter().filter(|l| is_hi(l.0)).count();
        (correct, e.current.members.len())
    }).unwrap_or((0, 0));

    let (off_correct, off_total) = off_entity.map(|e| {
        let correct = e.current.members.iter().filter(|l| is_officer(l.0)).count();
        (correct, e.current.members.len())
    }).unwrap_or((0, 0));

    let total_correct = hi_correct + off_correct;
    let overall_acc   = total_correct as f32 / 34.0 * 100.0;

    println!("  Results:");
    println!("    Entities detected : {n_entities}");
    println!("    Mr. Hi  entity    : {hi_correct}/{hi_total} nodes correct ({:.0}%)", hi_correct as f32 / hi_total.max(1) as f32 * 100.0);
    println!("    Officer entity    : {off_correct}/{off_total} nodes correct ({:.0}%)", off_correct as f32 / off_total.max(1) as f32 * 100.0);
    println!("    Overall accuracy  : {total_correct}/34 ({overall_acc:.1}%)");

    let prediction = format!(
        "Detected {n_entities} entit{}. \
         Mr. Hi cluster: {hi_total} nodes, {hi_correct} correctly from Mr. Hi's faction ({:.0}% purity). \
         Officer cluster: {off_total} nodes, {off_correct} correctly from Officer's faction ({:.0}% purity). \
         Overall: {total_correct}/34 nodes correctly assigned.",
        if n_entities == 1 { "y" } else { "ies" },
        hi_correct as f32 / hi_total.max(1) as f32 * 100.0,
        off_correct as f32 / off_total.max(1) as f32 * 100.0,
    );

    let ground_truth =
        "Two factions split the club: Mr. Hi (node 0) led 16 members \
         (nodes 0,1,2,3,4,5,6,7,8,10,11,12,13,17,19,21) and the Officer (node 33) \
         led 18 members (nodes 9,14,15,16,18,20,22,23,24,25,26,27,28,29,30,31,32,33). \
         The split was caused by a personal conflict; no node belongs to both groups.";

    let metrics = format!(
        "Mr. Hi accuracy: {hi_correct}/{} = {:.0}%; \
         Officer accuracy: {off_correct}/{} = {:.0}%; \
         Overall: {total_correct}/34 = {overall_acc:.1}%",
        MR_HI.len(), hi_correct as f32 / MR_HI.len() as f32 * 100.0,
        OFFICER.len(), off_correct as f32 / OFFICER.len() as f32 * 100.0,
    );

    println!("\n  LLM evaluation:");
    match score_prediction(client,
        "Predict the faction each club member belongs to, using only interaction dynamics \
         (no faction labels provided). Two factions are known from historical record.",
        ground_truth, &prediction, &metrics)
    {
        Ok(eval) => println!("{}", indent(&eval, "    ")),
        Err(e)   => println!("    [error] {e}"),
    }
    println!();
}

// ─── Task 2: Link prediction (held-out edges) ─────────────────────────────────

#[cfg(feature = "ollama")]
fn task2_link_prediction(client: &graph_llm::OllamaClient) {
    use graph_llm::score_prediction;
    use graph_core::{
        BatchId, Change, ChangeSubject, Endpoints, InfluenceKindId, Locus, LocusContext,
        LocusId, LocusKindId, LocusProgram, ProposedChange, Relationship, RelationshipLineage,
        StateVector,
    };
    use graph_engine::{
        DefaultEmergencePerspective, Engine, InfluenceKindConfig,
        InfluenceKindRegistry, LocusKindRegistry,
    };
    use graph_world::World;

    println!("━━━ Task 2: Link prediction (held-out edges) ━━━");
    println!("  Setup  : 78 − 10 = 68 training edges; 10 held-out intra-faction edges");
    println!("  Method : static entity detection on training graph");
    println!("  Scoring: held-out edge = predicted if both endpoints in same entity\n");

    const SOCIAL: InfluenceKindId = InfluenceKindId(1);
    const MEMBER: LocusKindId     = LocusKindId(1);

    struct Noop;
    impl LocusProgram for Noop {
        fn process(&self, _: &Locus, _: &[&Change], _: &dyn LocusContext) -> Vec<ProposedChange> { vec![] }
    }

    let training_edges: Vec<(u64, u64)> = ALL_EDGES.iter()
        .copied()
        .filter(|e| !HOLDOUT_EDGES.contains(e))
        .collect();

    let mut world = World::new();
    for i in 0..34u64 {
        world.insert_locus(Locus::new(LocusId(i), MEMBER, StateVector::zeros(1)));
    }
    for &(a, b) in &training_edges {
        let id  = world.relationships_mut().mint_id();
        let act = edge_activity(a, b, &training_edges);
        world.relationships_mut().insert(Relationship {
            id, kind: SOCIAL,
            endpoints: Endpoints::Symmetric { a: LocusId(a), b: LocusId(b) },
            state: StateVector::from_slice(&[act, 0.0]),
            lineage: RelationshipLineage::new_synthetic(SOCIAL),
            created_batch: BatchId(0), last_decayed_batch: 0, metadata: None,
        });
    }

    let mut loci_reg = LocusKindRegistry::new();
    loci_reg.insert(MEMBER, Box::new(Noop));
    let mut inf_reg = InfluenceKindRegistry::new();
    inf_reg.insert(SOCIAL, InfluenceKindConfig::new("social_tie").with_symmetric(true).with_decay(1.0));

    let engine  = Engine::default();
    let persp   = DefaultEmergencePerspective::default();
    engine.recognize_entities(&mut world, &inf_reg, &persp);

    let entity_of = |id: u64| -> Option<usize> {
        world.entities().active()
            .position(|e| e.current.members.contains(&LocusId(id)))
    };
    let same_entity = |a: u64, b: u64| entity_of(a).is_some() && entity_of(a) == entity_of(b);

    let holdout_hits: usize = HOLDOUT_EDGES.iter().filter(|&&(a, b)| same_entity(a, b)).count();
    let tn_hits:      usize = TRUE_NEGATIVES.iter().filter(|&&(a, b)| same_entity(a, b)).count();

    let precision = holdout_hits as f32 / HOLDOUT_EDGES.len() as f32 * 100.0;
    let tn_acc    = (TRUE_NEGATIVES.len() - tn_hits) as f32 / TRUE_NEGATIVES.len() as f32 * 100.0;

    println!("  Results:");
    println!("    Held-out edges predicted  : {holdout_hits}/{} ({precision:.0}% recall)", HOLDOUT_EDGES.len());
    println!("    True negatives rejected   : {}/{} ({tn_acc:.0}% specificity)", TRUE_NEGATIVES.len() - tn_hits, TRUE_NEGATIVES.len());

    let holdout_detail: Vec<String> = HOLDOUT_EDGES.iter().map(|&(a, b)| {
        format!("  ({a},{b}) → {}", if same_entity(a, b) { "✓ predicted" } else { "✗ missed" })
    }).collect();
    let tn_detail: Vec<String> = TRUE_NEGATIVES.iter().map(|&(a, b)| {
        format!("  ({a},{b}) → {}", if !same_entity(a, b) { "✓ correctly rejected" } else { "✗ false positive" })
    }).collect();

    println!("\n  Held-out edges:");
    for line in &holdout_detail { println!("   {line}"); }
    println!("\n  True negatives:");
    for line in &tn_detail { println!("   {line}"); }

    let prediction = format!(
        "Of 10 held-out intra-faction edges, {holdout_hits} were correctly predicted \
         (both endpoints landed in the same entity). Of 5 true-negative cross-faction \
         pairs, {} were correctly not predicted.",
        TRUE_NEGATIVES.len() - tn_hits
    );

    let ground_truth =
        "All 10 held-out edges connect two nodes within the same faction. \
         All 5 true-negative pairs connect nodes from different factions. \
         A perfect predictor would identify all 10 and reject all 5.";

    let metrics = format!(
        "Recall (held-out hits): {holdout_hits}/10 = {precision:.0}%; \
         Specificity (TN correct): {}/{} = {tn_acc:.0}%",
        TRUE_NEGATIVES.len() - tn_hits, TRUE_NEGATIVES.len()
    );

    println!("\n  LLM evaluation:");
    match score_prediction(client,
        "Predict which pairs of nodes will have an edge, given a graph with 10 edges removed. \
         A pair is predicted to link if both nodes end up in the same emergent entity.",
        ground_truth, &prediction, &metrics)
    {
        Ok(eval) => println!("{}", indent(&eval, "    ")),
        Err(e)   => println!("    [error] {e}"),
    }
    println!();
}

// ─── Task 3: Weight trend forecasting ────────────────────────────────────────

#[cfg(feature = "ollama")]
fn task3_trend_forecasting(client: &graph_llm::OllamaClient) {
    use graph_llm::score_prediction;
    use graph_core::{
        BatchId, ChangeSubject, Endpoints, InfluenceKindId, Locus, LocusId, LocusKindId,
        ProposedChange, Relationship, RelationshipLineage, StateVector,
    };
    use graph_engine::{
        Engine, EngineConfig, InfluenceKindConfig, InfluenceKindRegistry, LocusKindRegistry,
        PlasticityConfig,
    };
    use graph_query::{relationship_weight_trend_delta, Trend};
    use graph_world::World;

    println!("━━━ Task 3: Weight trend forecasting ━━━");
    println!("  Setup  : 78 edges, 200 ticks of BCM plasticity (η=0.5 τ=3, alternating stimulation)");
    println!("  Method : relationship_weight_trend_delta (ChangeLog first-vs-last on slot 1, threshold=0.003)");
    println!("  Oracle : intra-faction edges should trend Rising; cross-faction Stable/Falling\n");

    const SOCIAL: InfluenceKindId = InfluenceKindId(1);
    const MEMBER: LocusKindId     = LocusKindId(1);

    let mut world = World::new();
    for i in 0..34u64 {
        world.insert_locus(Locus::new(LocusId(i), MEMBER, StateVector::zeros(1)));
    }
    // Map from (a, b) → RelationshipId for trend lookup
    let mut edge_to_rel: std::collections::HashMap<(u64, u64), graph_core::RelationshipId> =
        std::collections::HashMap::new();
    for &(a, b) in ALL_EDGES {
        let id  = world.relationships_mut().mint_id();
        let act = edge_activity(a, b, ALL_EDGES);
        world.relationships_mut().insert(Relationship {
            id, kind: SOCIAL,
            endpoints: Endpoints::Symmetric { a: LocusId(a), b: LocusId(b) },
            state: StateVector::from_slice(&[act, 0.0]),
            lineage: RelationshipLineage::new_synthetic(SOCIAL),
            created_batch: BatchId(0), last_decayed_batch: 0, metadata: None,
        });
        edge_to_rel.insert((a, b), id);
    }

    let mut loci_reg = LocusKindRegistry::new();
    loci_reg.insert(MEMBER, Box::new(SpreadProgram));
    let mut inf_reg = InfluenceKindRegistry::new();
    inf_reg.insert(SOCIAL, InfluenceKindConfig::new("social_tie")
        .with_symmetric(true)
        .with_decay(0.99)
        .with_plasticity(PlasticityConfig { learning_rate: 0.5, max_weight: 10.0, weight_decay: 0.995, ..Default::default() }
            .with_bcm(3.0)));

    let engine = Engine::new(EngineConfig { max_batches_per_tick: 3 });

    let start_batch = world.current_batch();
    // Alternate stimulation: even ticks → locus 0 (Hi leader), odd ticks → locus 33 (Officer leader).
    // Anti-correlated activation lets BCM suppress cross-faction weights (post < θ_M → LTD).
    for tick in 0..200u64 {
        let leader = if tick % 2 == 0 { LocusId(0) } else { LocusId(33) };
        let stimuli = vec![
            ProposedChange::new(ChangeSubject::Locus(leader), SOCIAL, StateVector::from_slice(&[1.0])),
        ];
        engine.tick(&mut world, &loci_reg, &inf_reg, stimuli);
    }
    let end_batch = world.current_batch();

    // Classify trends per edge category using ChangeLog-backed OLS
    let mut intra_hi_trends    = [0usize; 3]; // [Rising, Stable, Falling]
    let mut intra_off_trends   = [0usize; 3];
    let mut cross_trends       = [0usize; 3];

    let trend_idx = |t: &Trend| match t {
        Trend::Rising { .. }  => 0,
        Trend::Stable         => 1,
        Trend::Falling { .. } => 2,
    };

    for &(a, b) in ALL_EDGES {
        let Some(&rel_id) = edge_to_rel.get(&(a, b)) else { continue };
        let trend = relationship_weight_trend_delta(
                &world, rel_id, start_batch, end_batch, 0.003)
            .unwrap_or(Trend::Stable);
        let ti = trend_idx(&trend);
        if is_hi(a) && is_hi(b)         { intra_hi_trends[ti]  += 1; }
        else if is_officer(a) && is_officer(b) { intra_off_trends[ti] += 1; }
        else                             { cross_trends[ti]      += 1; }
    }

    let intra_hi_total  = intra_hi_trends.iter().sum::<usize>();
    let intra_off_total = intra_off_trends.iter().sum::<usize>();
    let cross_total     = cross_trends.iter().sum::<usize>();

    let pct = |n: usize, d: usize| if d == 0 { 0.0 } else { n as f32 / d as f32 * 100.0 };

    println!("  Results (200 ticks alternating BCM η=0.5 τ=3, threshold=0.003, ChangeLog first-vs-last):");
    println!("  ┌──────────────────┬─────────┬─────────┬──────────┐");
    println!("  │ Edge type        │ Rising  │ Stable  │ Falling  │");
    println!("  ├──────────────────┼─────────┼─────────┼──────────┤");
    println!("  │ Intra-Hi ({:2})   │ {:3.0}%    │ {:3.0}%    │ {:3.0}%     │",
        intra_hi_total,
        pct(intra_hi_trends[0], intra_hi_total),
        pct(intra_hi_trends[1], intra_hi_total),
        pct(intra_hi_trends[2], intra_hi_total));
    println!("  │ Intra-Off ({:2})  │ {:3.0}%    │ {:3.0}%    │ {:3.0}%     │",
        intra_off_total,
        pct(intra_off_trends[0], intra_off_total),
        pct(intra_off_trends[1], intra_off_total),
        pct(intra_off_trends[2], intra_off_total));
    println!("  │ Cross ({:2})      │ {:3.0}%    │ {:3.0}%    │ {:3.0}%     │",
        cross_total,
        pct(cross_trends[0], cross_total),
        pct(cross_trends[1], cross_total),
        pct(cross_trends[2], cross_total));
    println!("  └──────────────────┴─────────┴─────────┴──────────┘");

    let prediction = format!(
        "Over 200 simulation ticks (BCM plasticity η=0.5 τ=3, alternating leader stimulation, decay=0.99), using ChangeLog first-vs-last on weight slot: \
         {:.0}% of intra-Hi edges and {:.0}% of intra-Officer edges trend Rising; \
         {:.0}% of cross-faction edges are Falling or Stable.",
        pct(intra_hi_trends[0], intra_hi_total),
        pct(intra_off_trends[0], intra_off_total),
        pct(cross_trends[1] + cross_trends[2], cross_total),
    );

    let ground_truth =
        "Hebbian plasticity reinforces intra-faction edges (co-activated by the same leader \
         seed at nodes 0 and 33) and leaves cross-faction bridges comparatively less reinforced. \
         Expectation: most intra-faction edges trend Rising; cross-faction edges trend \
         Stable or Falling as relative activity concentrates within clusters.";

    let metrics = format!(
        "Intra-Hi Rising: {:.0}%; Intra-Officer Rising: {:.0}%; \
         Cross Rising: {:.0}%; Cross Falling+Stable: {:.0}%",
        pct(intra_hi_trends[0], intra_hi_total),
        pct(intra_off_trends[0], intra_off_total),
        pct(cross_trends[0], cross_total),
        pct(cross_trends[1] + cross_trends[2], cross_total),
    );

    println!("\n  LLM evaluation:");
    match score_prediction(client,
        "Forecast whether each edge's Hebbian weight will rise or fall under continued dynamics. \
         Intra-faction edges should strengthen; cross-faction edges should weaken.",
        ground_truth, &prediction, &metrics)
    {
        Ok(eval) => println!("{}", indent(&eval, "    ")),
        Err(e)   => println!("    [error] {e}"),
    }
    println!();
}
