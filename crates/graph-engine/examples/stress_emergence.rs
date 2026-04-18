//! Stress-emergence benchmark — Entity split/merge heavy workload.
//!
//! Phase 1 (E1) measurement for the roadmap. Exercises
//! `EmergencePerspective::recognize` at scale by driving continuous entity
//! split/merge through a phase-rotating stimulus pattern.
//!
//! ## Design
//!
//! - `--size N` (default 100) controls the number of loci. Three "community"
//!   clusters of N/3 nodes each, connected by sparse random cross-links.
//! - `--batches N` (default 20) controls how many outer loop iterations
//!   (each = one `sim.step()` + `recognize_entities`) are run.
//! - Hebbian plasticity + weight decay cause relationship strengths to shift
//!   continuously. Phase-rotating stimulus (each batch targets a different
//!   random subset) ensures old relationships decay out and new ones emerge,
//!   generating split/merge events across multiple batches.
//!
//! ## Output (CSV to stdout)
//!
//! ```
//! batch,entities,relationships,ms
//! 1,4,87,2
//! 2,6,91,3
//! ...
//! ```
//!
//! Run:
//! ```
//! cargo run -p graph-engine --example stress_emergence -- --size 100 --batches 20
//! cargo run -p graph-engine --example stress_emergence -- --size 1000 --batches 20
//! cargo run -p graph-engine --example stress_emergence -- --size 10000 --batches 20
//! ```

use std::sync::Arc;
use std::time::Instant;

use graph_core::{
    Change, Endpoints, InfluenceKindId, Locus, LocusContext, LocusId, LocusKindId,
    LocusProgram, ProposedChange, StateVector, StructuralProposal,
};
use graph_engine::{
    DefaultCoherePerspective, DefaultEmergencePerspective, DemotionPolicy, EngineConfig,
    InfluenceKindConfig, InfluenceKindRegistry, LocusKindConfig, LocusKindRegistry,
    PlasticityConfig, Simulation, SimulationConfig,
};
use graph_world::World;

// ── Kind IDs ──────────────────────────────────────────────────────────────────

const KIND_NODE: LocusKindId = LocusKindId(1);
const INF_EXCITE: InfluenceKindId = InfluenceKindId(1);

/// Connections with weight below this are pruned by the structural proposals.
const PRUNE_WEIGHT: f32 = 0.005;

// ── Deterministic RNG ─────────────────────────────────────────────────────────

struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self { Self(seed) }
    fn next(&mut self) -> u64 {
        self.0 = self.0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0
    }
    fn range(&mut self, lo: u64, hi: u64) -> u64 {
        lo + self.next() % (hi - lo)
    }
    fn f32(&mut self) -> f32 {
        (self.next() >> 11) as f32 / (1u64 << 53) as f32
    }
}

// ── Topology ──────────────────────────────────────────────────────────────────

/// Sparse adjacency list (undirected, stored as directed both ways).
struct Topology {
    /// neighbours[i] = list of locus IDs that locus i can fire to.
    neighbours: Vec<Vec<LocusId>>,
    n: usize,
}

/// Build a ring-of-rings topology:
///
/// - N loci split into `num_clusters` clusters.
/// - Within each cluster, each node is connected to its k nearest neighbours
///   on the ring (k = 4 for small N, capped by cluster size).
/// - `cross_links` random cross-cluster edges are added to allow merges.
fn build_topology(n: usize, seed: u64) -> Topology {
    let mut rng = Rng::new(seed);
    let mut adj: Vec<Vec<LocusId>> = vec![Vec::new(); n];

    let num_clusters = 3usize.min(n);
    let cluster_size = n / num_clusters;

    // Intra-cluster k-nearest ring connections.
    let k = 2usize.min(cluster_size.saturating_sub(1));
    for c in 0..num_clusters {
        let base = c * cluster_size;
        let end = if c + 1 == num_clusters { n } else { base + cluster_size };
        let sz = end - base;
        if sz < 2 { continue; }
        for i in 0..sz {
            for d in 1..=k {
                let j = (i + d) % sz;
                let a = LocusId((base + i) as u64);
                let b = LocusId((base + j) as u64);
                if !adj[a.0 as usize].contains(&b) {
                    adj[a.0 as usize].push(b);
                }
                if !adj[b.0 as usize].contains(&a) {
                    adj[b.0 as usize].push(a);
                }
            }
        }
    }

    // Cross-cluster random edges: ~sqrt(N) connections.
    let cross_links = (n as f64).sqrt() as usize + 2;
    for _ in 0..cross_links {
        let a = rng.range(0, n as u64) as usize;
        let b = rng.range(0, n as u64) as usize;
        if a == b { continue; }
        let aid = LocusId(a as u64);
        let bid = LocusId(b as u64);
        if !adj[a].contains(&bid) {
            adj[a].push(bid);
        }
        if !adj[b].contains(&aid) {
            adj[b].push(aid);
        }
    }

    Topology { neighbours: adj, n }
}

// ── Program ───────────────────────────────────────────────────────────────────

struct NodeProgram {
    topo: Arc<Topology>,
}

impl LocusProgram for NodeProgram {
    fn process(
        &self,
        locus: &Locus,
        incoming: &[&Change],
        _ctx: &dyn LocusContext,
    ) -> Vec<ProposedChange> {
        // Sum incoming signal.
        let signal: f32 = incoming
            .iter()
            .filter(|c| c.kind == INF_EXCITE)
            .map(|c| c.after.as_slice().iter().copied().sum::<f32>())
            .sum();

        if signal < 0.05 {
            return vec![];
        }

        let idx = locus.id.0 as usize;
        if idx >= self.topo.n { return vec![]; }

        let out = (signal * 0.6).min(1.0);
        self.topo.neighbours[idx]
            .iter()
            .map(|&t| ProposedChange::stimulus(t, INF_EXCITE, &[out]))
            .collect()
    }

    fn structural_proposals(
        &self,
        locus: &Locus,
        _incoming: &[&Change],
        ctx: &dyn LocusContext,
    ) -> Vec<StructuralProposal> {
        // Prune the single weakest incoming edge when it drops below threshold.
        let mut weakest: Option<(graph_core::RelationshipId, f32)> = None;
        for r in ctx.relationships_for(locus.id) {
            let is_incoming = matches!(
                &r.endpoints,
                Endpoints::Directed { to, .. } if *to == locus.id
            );
            if is_incoming {
                let w = r.weight();
                if w < PRUNE_WEIGHT {
                    if weakest.is_none_or(|(_, ww)| w < ww) {
                        weakest = Some((r.id, w));
                    }
                }
            }
        }
        if let Some((rid, _)) = weakest {
            vec![StructuralProposal::DeleteRelationship { rel_id: rid }]
        } else {
            vec![]
        }
    }
}

// ── World / registry construction ────────────────────────────────────────────

fn build_world_and_registries(
    n: usize,
    topo: Arc<Topology>,
    demotion: Option<DemotionPolicy>,
) -> (World, LocusKindRegistry, InfluenceKindRegistry) {
    let mut world = World::new();
    for i in 0..n {
        world.insert_locus(Locus::new(LocusId(i as u64), KIND_NODE, StateVector::zeros(1)));
    }

    let mut loci = LocusKindRegistry::new();
    loci.insert_with_config(KIND_NODE, LocusKindConfig {
        name: None,
        state_slots: Vec::new(),
        program: Box::new(NodeProgram { topo }),
        refractory_batches: 2,
        encoder: None,
        max_proposals_per_dispatch: None,
    });

    let mut influences = InfluenceKindRegistry::new();
    let mut cfg = InfluenceKindConfig::new("excitatory")
        .with_decay(0.80)
        .with_plasticity(PlasticityConfig {
            learning_rate: 0.02,
            weight_decay: 0.990,
            max_weight: 3.0,
            stdp: false,
            ..Default::default()
        })
        .with_prune_threshold(PRUNE_WEIGHT);
    if let Some(policy) = demotion {
        cfg = cfg.with_demotion(policy);
    }
    influences.insert(INF_EXCITE, cfg);

    (world, loci, influences)
}

// ── Stimulus generation ───────────────────────────────────────────────────────

/// Stimulate `count` random loci in a random cluster (chosen by `batch` index).
/// The cluster cycles each batch so that phase-rotation drives split/merge.
fn phase_stimulus(rng: &mut Rng, n: usize, batch: usize, count: usize) -> Vec<ProposedChange> {
    if n == 0 || count == 0 { return vec![]; }

    let num_clusters = 3usize.min(n);
    let cluster_size = n / num_clusters;
    // Rotate through clusters + introduce occasional cross-cluster disruption.
    let primary_cluster = batch % num_clusters;
    let base = primary_cluster * cluster_size;
    let end = ((primary_cluster + 1) * cluster_size).min(n);
    let sz = end - base;
    if sz == 0 { return vec![]; }

    let mut stimuli = Vec::with_capacity(count);
    for _ in 0..count {
        let offset = rng.range(0, sz as u64) as usize;
        let id = LocusId((base + offset) as u64);
        let intensity = 0.4 + 0.4 * rng.f32();
        stimuli.push(ProposedChange::stimulus(id, INF_EXCITE, &[intensity]));
    }

    // Every 3rd batch, also fire a disruptor burst into a different cluster
    // to cause cross-cluster activity that drives merges then splits.
    if batch % 3 == 2 {
        let disruptor_cluster = (primary_cluster + 1) % num_clusters;
        let dbase = disruptor_cluster * cluster_size;
        let dend = ((disruptor_cluster + 1) * cluster_size).min(n);
        let dsz = dend - dbase;
        if dsz > 0 {
            let burst = (count / 2).max(1);
            for _ in 0..burst {
                let offset = rng.range(0, dsz as u64) as usize;
                let id = LocusId((dbase + offset) as u64);
                let intensity = 0.6 + 0.3 * rng.f32();
                stimuli.push(ProposedChange::stimulus(id, INF_EXCITE, &[intensity]));
            }
        }
    }

    stimuli
}

// ── CLI parsing ───────────────────────────────────────────────────────────────

struct Args {
    size: usize,
    batches: usize,
    /// None = no demotion (baseline); Some = ActivityFloor policy with given floor value.
    demotion_floor: Option<f32>,
    /// Burst-quiet mode: stimulate for `burst_on` batches then go silent for `burst_off` batches.
    burst_on: Option<usize>,
    burst_off: Option<usize>,
    /// When true, print an `EmergenceReport` in markdown to stderr on exit.
    /// H4 (roadmap Track H) uses this to audit Ψ distributions per workload.
    psi: bool,
}

fn parse_args() -> Args {
    let args: Vec<String> = std::env::args().collect();
    let mut size = 100usize;
    let mut batches = 20usize;
    let mut demotion_floor: Option<f32> = None;
    let mut burst_on: Option<usize> = None;
    let mut burst_off: Option<usize> = None;
    let mut psi = false;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--size" if i + 1 < args.len() => {
                size = args[i + 1].parse().expect("--size must be a positive integer");
                i += 2;
            }
            "--batches" if i + 1 < args.len() => {
                batches = args[i + 1].parse().expect("--batches must be a positive integer");
                i += 2;
            }
            "--demotion-floor" if i + 1 < args.len() => {
                demotion_floor = Some(args[i + 1].parse().expect("--demotion-floor must be a float"));
                i += 2;
            }
            "--burst-on" if i + 1 < args.len() => {
                burst_on = Some(args[i + 1].parse().expect("--burst-on must be a positive integer"));
                i += 2;
            }
            "--burst-off" if i + 1 < args.len() => {
                burst_off = Some(args[i + 1].parse().expect("--burst-off must be a positive integer"));
                i += 2;
            }
            "--psi" => {
                psi = true;
                i += 1;
            }
            other => {
                eprintln!("Unknown argument: {other}");
                std::process::exit(1);
            }
        }
    }
    Args { size, batches, demotion_floor, burst_on, burst_off, psi }
}

// ── Main ──────────────────────────────────────────────────────────────────────

fn main() {
    let args = parse_args();
    let n = args.size.max(3); // need at least 3 nodes
    let num_batches = args.batches;

    let demotion_policy = args.demotion_floor.map(DemotionPolicy::ActivityFloor);

    let t_build = Instant::now();
    let topo = Arc::new(build_topology(n, 42));
    let (world, loci, influences) = build_world_and_registries(n, Arc::clone(&topo), demotion_policy);

    let mut sim = Simulation::with_config(
        world,
        loci,
        influences,
        SimulationConfig {
            engine: EngineConfig {
                max_batches_per_tick: 16,
            },
            ..Default::default()
        },
    );

    let build_ms = t_build.elapsed().as_millis();
    let demotion_label = match demotion_policy {
        Some(DemotionPolicy::ActivityFloor(f)) => format!("ActivityFloor({f:.3})"),
        Some(DemotionPolicy::IdleBatches(n)) => format!("IdleBatches({n})"),
        Some(DemotionPolicy::LruCapacity(c)) => format!("LruCapacity({c})"),
        None => "none".to_string(),
    };
    let burst_label = match (args.burst_on, args.burst_off) {
        (Some(on), Some(off)) => format!("on={on}/off={off}"),
        _ => "continuous".to_string(),
    };
    eprintln!(
        "stress_emergence: N={n} batches={num_batches} demotion={demotion_label} stimulus={burst_label} build={build_ms}ms"
    );

    // Emergence perspective: lower threshold so relationships emerge quickly
    // at startup, driving entity formation from the first few ticks.
    let ep = DefaultEmergencePerspective {
        min_activity_threshold: 0.05,
        overlap_threshold: 0.2,
    };
    let cp = DefaultCoherePerspective {
        min_bridge_activity: 0.05,
        ..Default::default()
    };

    let mut rng = Rng::new(1337);
    // Number of loci to stimulate per batch: ~10% of N, at least 1.
    let stim_count = (n / 10).max(1);

    // CSV header.
    println!("batch,entities,relationships,ms");

    for batch_idx in 0..num_batches {
        let t_start = Instant::now();

        // Burst-quiet mode: silent during the "off" phase of each cycle.
        let stimuli = match (args.burst_on, args.burst_off) {
            (Some(on), Some(off)) => {
                let cycle = on + off;
                let pos = batch_idx % cycle;
                if pos < on {
                    phase_stimulus(&mut rng, n, batch_idx, stim_count)
                } else {
                    vec![]
                }
            }
            _ => phase_stimulus(&mut rng, n, batch_idx, stim_count),
        };
        let _obs = sim.step(stimuli);

        // Recognize entities every batch to maximise split/merge events.
        sim.recognize_entities(&ep);
        sim.extract_cohere(&cp);

        let elapsed_ms = t_start.elapsed().as_millis();

        let world = sim.world();
        let entity_count = world.entities().iter().count();
        let rel_count = world.relationships().iter().count();
        drop(world);

        println!("{},{},{},{}", batch_idx + 1, entity_count, rel_count, elapsed_ms);
    }

    // Final summary to stderr so it doesn't pollute CSV stdout.
    let world = sim.world();
    let active = world.entities().active_count();
    let total_ents = world.entities().iter().count();
    let total_rels = world.relationships().iter().count();
    let total_changes = world.log().iter().count();
    drop(world);

    eprintln!(
        "done: entities={total_ents} (active={active}) relationships={total_rels} changes={total_changes}"
    );

    // Count split/merge from BatchId(0) range.
    {
        let world = sim.world();
        let mut splits = 0usize;
        let mut merges = 0usize;
        let mut born = 0usize;
        let mut dormant = 0usize;
        for e in world.entities().iter() {
            for layer in &e.layers {
                match &layer.transition {
                    graph_core::LayerTransition::Born => born += 1,
                    graph_core::LayerTransition::Split { .. } => splits += 1,
                    graph_core::LayerTransition::Merged { .. } => merges += 1,
                    graph_core::LayerTransition::BecameDormant => dormant += 1,
                    _ => {}
                }
            }
        }
        eprintln!("lifecycle: born={born} splits={splits} merges={merges} dormant={dormant}");
    }

    // Partition within/cross ratio — E4 feasibility check.
    // Measures both edge count and touch-weighted (change_count) locality.
    {
        let world = sim.world();
        let p = 10usize;
        let mut within_edges: usize = 0;
        let mut cross_edges: usize = 0;
        let mut within_touches: u64 = 0;
        let mut cross_touches: u64 = 0;
        for rel in world.relationships().iter() {
            let (a, b) = match &rel.endpoints {
                graph_core::Endpoints::Directed { from, to } => (*from, *to),
                graph_core::Endpoints::Symmetric { a, b } => (*a, *b),
            };
            let pa = (a.0 as usize).saturating_mul(p) / n;
            let pb = (b.0 as usize).saturating_mul(p) / n;
            let touches = rel.lineage.change_count as u64;
            if pa == pb {
                within_edges += 1;
                within_touches += touches;
            } else {
                cross_edges += 1;
                cross_touches += touches;
            }
        }
        let total_edges = within_edges + cross_edges;
        let total_touches = within_touches + cross_touches;
        let edge_pct = if total_edges > 0 { within_edges * 100 / total_edges } else { 0 };
        let touch_pct = if total_touches > 0 { within_touches * 100 / total_touches } else { 0 };
        eprintln!(
            "partition(P={p},N={n}): edges within%={edge_pct}% touches within%={touch_pct}% (edges={total_edges} touches={total_touches})"
        );
    }

    // H4 — optional Ψ (emergence capacity) audit. Printed to stderr in
    // markdown so CSV output on stdout stays machine-parseable.
    if args.psi {
        let decay = sim.activity_decay_rates();
        let world = sim.world();
        let report = graph_query::emergence_report_with_decay(&world, &decay);
        eprintln!("\n{}", report.render_markdown());
        let synergy = graph_query::emergence_report_synergy_with_decay(&world, &decay);
        eprintln!("\n{}", synergy.render_markdown());

        // H4.2 — leave-one-out robustness for any entity with
        // `psi_pair_top3 > 0`. Expected to be rare on this workload
        // (fifth pass saw Entity 73 at b=50 as the sole such entity).
        for entry in synergy.emergent.iter().chain(synergy.spurious.iter()) {
            if entry.psi.psi_pair_top3 > 0.0 {
                if let Some(loo) = graph_query::psi_synergy_leave_one_out_with_decay(
                    &world, entry.entity, &decay,
                ) {
                    eprintln!("\n{}", loo.render_markdown());
                }
            }
        }
    }
}
