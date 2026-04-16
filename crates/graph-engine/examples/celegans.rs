//! C. elegans connectome simulation.
//!
//! Loads the real *C. elegans* hermaphrodite neural connectome (Varshney et al. 2011,
//! re-processed by the CElegansTP project) and runs a substrate simulation over it.
//!
//! ## Data
//!
//! Two CSV files in `examples/data/`:
//!
//! | File | Contents |
//! |------|----------|
//! | `celegans_connectome.csv` | `Neuron,Target,Connections,NT` — directed synapses |
//! | `celegans_sensory.csv`    | `Function,Neuron,Weight,NT`   — sensory neuron metadata |
//!
//! Neurotransmitter labels in the connectome file:
//! - `exc` → excitatory (`activity_contribution = +1.0`)
//! - `inh` → inhibitory (`activity_contribution = -1.0`)
//!
//! ## Simulation design
//!
//! - Each neuron becomes a **Locus** with a 1-slot state (membrane potential proxy).
//! - Three influence kinds: `excitatory`, `inhibitory`, `sensory`.
//! - `LocusProgram`: integrate incoming signal, fire to synaptic targets if above threshold.
//! - **Stimulation**: 3 rounds targeting known sensory classes:
//!   1. Touch (PLM, ALM) — posterior/anterior touch → locomotion
//!   2. Nociception (ASH) — harsh touch/chemical → avoidance
//!   3. Chemosensory (AWC, AWA) — olfaction → approach/avoidance
//! - After each round: `recognize_entities` to observe circuit emergence.
//!
//! ## What to look for
//!
//! - **Touch circuit** (PLM/ALM → AVC/PVC → AVB/AVD → motor neurons) should emerge
//!   as a coherent entity.
//! - **Avoidance circuit** (ASH → AVA/AVD → A-type motors) should form a separate entity
//!   or cluster within a cohere.
//! - **Inhibitory interneurons** (RIS, DVB, VD/DD motor neurons) should sit at
//!   entity *boundaries* — the new repulsion-aware clustering should prevent them
//!   from being merged into the excitatory clusters they gate.
//!
//! Run:
//! ```
//! cargo run -p graph-engine --example celegans
//! ```

use std::collections::HashMap;
use std::sync::Arc;

use graph_core::{
    Change, Endpoints, InfluenceKindId, Locus, LocusContext, LocusId, LocusKindId,
    LocusProgram, Properties, ProposedChange, StateVector,
};
use graph_engine::{
    DefaultCoherePerspective, DefaultEmergencePerspective, InfluenceKindConfig,
    InfluenceKindRegistry, LocusKindRegistry, PlasticityConfig,
    SimulationConfig,
};
use graph_query::{
    NameMap,
    entity_deviations_since, entities_summary,
    relationships_absent_without,
    to_dot_named,
};

// ── Influence kind IDs ─────────────────────────────────────────────────────────

const KIND_EXC: InfluenceKindId = InfluenceKindId(1); // excitatory chemical synapse
const KIND_INH: InfluenceKindId = InfluenceKindId(2); // inhibitory chemical synapse
const KIND_SENS: InfluenceKindId = InfluenceKindId(3); // external sensory stimulus

// ── Locus kind IDs ────────────────────────────────────────────────────────────

const LKIND_NEURON: LocusKindId = LocusKindId(1);

// ── Neuron program ────────────────────────────────────────────────────────────

/// Leaky integrate-and-fire proxy.
///
/// All neurons share one program instance. Each locus looks up its own
/// synaptic targets from the shared `targets` map keyed by `LocusId`.
///
/// On each dispatch:
/// 1. Sum incoming signal (excitatory positive, inhibitory negative).
/// 2. If net input + current state > threshold, fire: forward a scaled signal
///    to each pre-wired target.
/// 3. Regardless of firing, apply a small leak (state decays toward 0).
struct NeuronProgram {
    /// `locus_id → [(target_id, weight, is_excitatory)]`
    targets: Arc<HashMap<LocusId, Vec<(LocusId, f32, bool)>>>,
    /// Fire threshold for the membrane potential proxy.
    threshold: f32,
    /// Fraction of state preserved each dispatch (leak).
    leak: f32,
}

impl LocusProgram for NeuronProgram {
    fn process(
        &self,
        locus: &Locus,
        incoming: &[&Change],
        _ctx: &dyn LocusContext,
    ) -> Vec<ProposedChange> {
        if incoming.is_empty() {
            return vec![];
        }

        // Net input: excitatory adds, inhibitory subtracts.
        let net_input: f32 = incoming.iter().map(|c| {
            let mag = c.after.as_slice().first().copied().unwrap_or(0.0);
            if c.kind == KIND_INH { -mag.abs() } else { mag }
        }).sum();

        let current = locus.state.as_slice().first().copied().unwrap_or(0.0);
        let membrane = (current * self.leak + net_input).clamp(-2.0, 2.0);

        let targets = match self.targets.get(&locus.id) {
            Some(t) if !t.is_empty() => t,
            _ => return vec![],
        };

        if membrane.abs() < self.threshold {
            return vec![];
        }

        // Fire: forward attenuated signal to each target.
        // The synapse weight scales the signal to control cascade propagation —
        // weak biological synapses (low count/max_count) send proportionally
        // smaller signals, preventing the entire network from firing in lockstep.
        // This means pre_signal = membrane * 0.5 * weight is small for weak
        // synapses, so Hebbian learning_rate is set high (0.5) to compensate.
        let signal = membrane * 0.5;
        targets.iter().map(|&(target, weight, is_exc)| {
            let kind = if is_exc { KIND_EXC } else { KIND_INH };
            ProposedChange::new(
                graph_core::ChangeSubject::Locus(target),
                kind,
                StateVector::from_slice(&[(signal * weight).clamp(-1.0, 1.0)]),
            )
        }).collect()
    }
}

// ── CSV parsing ───────────────────────────────────────────────────────────────

#[derive(Debug)]
struct SynapseRow {
    from: String,
    to: String,
    count: f32,
    nt: String, // "exc" or "inh"
}

#[derive(Debug)]
struct SensoryRow {
    function: String,
    neuron: String,
}

fn parse_connectome(csv: &str) -> Vec<SynapseRow> {
    csv.lines()
        .skip(1)
        .filter_map(|line| {
            let cols: Vec<&str> = line.splitn(6, ',').collect();
            // format: index, Neuron, Target, Number of Connections, Neurotransmitter
            if cols.len() < 5 { return None; }
            let from  = cols[1].trim().to_owned();
            let to    = cols[2].trim().to_owned();
            let count = cols[3].trim().parse::<f32>().ok()?;
            let nt    = cols[4].trim().to_lowercase();
            if from.is_empty() || to.is_empty() { return None; }
            Some(SynapseRow { from, to, count, nt })
        })
        .collect()
}

fn parse_sensory(csv: &str) -> Vec<SensoryRow> {
    csv.lines()
        .skip(1)
        .filter_map(|line| {
            let cols: Vec<&str> = line.splitn(5, ',').collect();
            // format: index, Function, Neuron, Weight, Neurotransmitter
            if cols.len() < 3 { return None; }
            let function = cols[1].trim().to_owned();
            let neuron   = cols[2].trim().to_owned();
            if neuron.is_empty() { return None; }
            Some(SensoryRow { function, neuron })
        })
        .collect()
}

// ── World construction ────────────────────────────────────────────────────────

fn build_simulation(
    connectome: &[SynapseRow],
    sensory_meta: &[SensoryRow],
) -> (graph_engine::Simulation, HashMap<String, LocusId>, HashMap<LocusId, String>) {
    // ── Assign locus IDs ──────────────────────────────────────────────────
    let mut name_to_id: HashMap<String, LocusId> = HashMap::new();
    let mut id_to_name: HashMap<LocusId, String> = HashMap::new();
    let mut next_id = 0u64;

    let get_or_insert = |name: &str,
                              name_to_id: &mut HashMap<String, LocusId>,
                              id_to_name: &mut HashMap<LocusId, String>,
                              next_id: &mut u64| -> LocusId {
        if let Some(&id) = name_to_id.get(name) {
            return id;
        }
        let id = LocusId(*next_id);
        *next_id += 1;
        name_to_id.insert(name.to_owned(), id);
        id_to_name.insert(id, name.to_owned());
        id
    };

    for row in connectome {
        get_or_insert(&row.from, &mut name_to_id, &mut id_to_name, &mut next_id);
        get_or_insert(&row.to,   &mut name_to_id, &mut id_to_name, &mut next_id);
    }

    let n_neurons = next_id as usize;
    println!("  Neurons: {n_neurons}");
    println!("  Synapses: {}", connectome.len());

    // ── Build target lists per neuron ────────────────────────────────────
    // max synapse count (for weight normalisation)
    let max_count = connectome.iter().map(|r| r.count).fold(0.0f32, f32::max).max(1.0);

    let mut targets_map: HashMap<LocusId, Vec<(LocusId, f32, bool)>> = HashMap::new();
    for row in connectome {
        let from_id = name_to_id[&row.from];
        let to_id   = name_to_id[&row.to];
        let weight  = (row.count / max_count).clamp(0.01, 1.0);
        let is_exc  = !row.nt.starts_with("inh");
        targets_map.entry(from_id).or_default().push((to_id, weight, is_exc));
    }

    // ── Registries ───────────────────────────────────────────────────────
    let mut loci_reg = LocusKindRegistry::new();
    let mut infl_reg = InfluenceKindRegistry::new();

    infl_reg.insert(KIND_EXC,  InfluenceKindConfig::new("excitatory")
        .with_decay(0.85)
        .with_activity_contribution(1.0)
        .with_max_activity(3.0)
        .with_plasticity(PlasticityConfig { learning_rate: 0.5, weight_decay: 0.995, max_weight: 2.0, stdp: false })
        .with_min_emerge_activity(0.05));
    infl_reg.insert(KIND_INH,  InfluenceKindConfig::new("inhibitory")
        .with_decay(0.85)
        .with_activity_contribution(-1.0)
        .with_max_activity(3.0)
        .with_min_emerge_activity(0.05));
    infl_reg.insert(KIND_SENS, InfluenceKindConfig::new("sensory")
        .with_decay(1.0)
        .with_activity_contribution(1.0)
        .with_max_activity(2.0));

    // All neurons share one program that looks up targets by locus.id at runtime.
    let shared_targets: Arc<HashMap<LocusId, Vec<(LocusId, f32, bool)>>> = Arc::new(targets_map);
    loci_reg.insert(LKIND_NEURON, Box::new(NeuronProgram {
        targets: Arc::clone(&shared_targets),
        threshold: 0.15,
        leak: 0.7,
    }));

    // ── World: insert all loci ───────────────────────────────────────────
    let mut world = graph_world::World::new();
    for (&id, _) in &id_to_name {
        world.insert_locus(Locus::new(id, LKIND_NEURON, StateVector::zeros(1)));
    }

    // ── Register names + sensory function tags in PropertyStore ─────────
    // Every neuron gets a "name" property so NameMap can resolve IDs to
    // human-readable labels without carrying a separate HashMap.
    for (&id, name) in &id_to_name {
        let mut props = Properties::new();
        props.set("name", name.clone());
        world.properties_mut().insert(id, props);
    }
    // Overlay sensory function tags on top of the existing name properties.
    for row in sensory_meta {
        if let Some(&id) = name_to_id.get(&row.neuron) {
            if let Some(props) = world.properties_mut().get_mut(id) {
                props.set("function", row.function.clone());
            }
        }
    }

    let sim_config = SimulationConfig::default();
    let sim = graph_engine::Simulation::with_config(world, loci_reg, infl_reg, sim_config);

    (sim, name_to_id, id_to_name)
}


// ── Stimulation helpers ───────────────────────────────────────────────────────

fn stimulate(
    sim: &mut graph_engine::Simulation,
    name_to_id: &HashMap<String, LocusId>,
    neurons: &[&str],
    magnitude: f32,
) -> graph_engine::StepObservation {
    let stimuli: Vec<ProposedChange> = neurons.iter()
        .filter_map(|&name| name_to_id.get(name))
        .map(|&id| ProposedChange::new(
            graph_core::ChangeSubject::Locus(id),
            KIND_SENS,
            StateVector::from_slice(&[magnitude]),
        ))
        .collect();
    sim.step(stimuli)
}

// ── Main ──────────────────────────────────────────────────────────────────────

fn main() {
    let connectome_csv = include_str!("data/celegans_connectome.csv");
    let sensory_csv    = include_str!("data/celegans_sensory.csv");

    let connectome   = parse_connectome(connectome_csv);
    let sensory_meta = parse_sensory(sensory_csv);

    println!("=== C. elegans Connectome Simulation ===\n");

    let (mut sim, name_to_id, id_to_name) = build_simulation(&connectome, &sensory_meta);

    // NameMap is built once from PropertyStore (all neurons have "name" set).
    let names = NameMap::from_world(&*sim.world());

    let ep = DefaultEmergencePerspective {
        min_activity_threshold: 0.05,
        overlap_threshold: 0.4,
    };
    let cp = DefaultCoherePerspective {
        min_bridge_activity: 0.03,
        ..Default::default()
    };

    // ── Round 1: Touch stimulation ────────────────────────────────────────
    println!("\n--- Round 1: Posterior touch (PLM, PVD) ---");
    // Snapshot the batch before stimulation so we can look up the first batch's
    // root changes for counterfactual analysis.
    let pre_r1_batch = sim.world().current_batch();
    for _ in 0..5 {
        stimulate(&mut sim, &name_to_id, &["PLML", "PLMR", "PVDL", "PVDR"], 0.8);
    }
    let post_r1_batch = sim.world().current_batch();
    // Root changes = all changes in the very first stimulation batch.
    // The engine commits pending changes using world.current_batch() at the
    // start of each loop iteration, then calls advance_batch() at the end.
    // So the first batch written during the first stimulate() call is
    // pre_r1_batch (not pre_r1_batch + 1).
    let r1_roots: Vec<_> = sim.world().log()
        .batch(pre_r1_batch)
        .map(|c| c.id)
        .collect();

    let events = sim.recognize_entities(&ep);
    print_emergence_events(&events, &id_to_name, &sim);
    print_entities(&sim, &id_to_name);

    // Counterfactual: which relationships were born from the touch stimulus?
    let absent_r1 = relationships_absent_without(&*sim.world(), &r1_roots);
    println!("  [counterfactual] {} relationship(s) would not exist without the initial touch stimulus", absent_r1.len());
    for rel_id in absent_r1.iter().take(5) {
        let wg = sim.world();
        if let Some(rel) = wg.relationships().get(*rel_id) {
            let (from, to) = match rel.endpoints {
                Endpoints::Directed { from, to } => (from, to),
                Endpoints::Symmetric { a, b } => (a, b),
            };
            println!("    {} → {}  activity={:.3}", names.name(from), names.name(to), rel.activity());
        }
    }
    if absent_r1.len() > 5 { println!("    … and {} more", absent_r1.len() - 5); }

    // ── Round 2: Nociception (harsh touch / chemical) ─────────────────────
    println!("\n--- Round 2: Nociception (ASH — harsh touch / chemical) ---");
    let pre_r2_batch = sim.world().current_batch();
    for _ in 0..5 {
        stimulate(&mut sim, &name_to_id, &["ASHL", "ASHR"], 1.0);
    }
    let post_r2_batch = sim.world().current_batch();
    let r2_roots: Vec<_> = sim.world().log()
        .batch(pre_r2_batch)
        .map(|c| c.id)
        .collect();
    let events = sim.recognize_entities(&ep);
    print_emergence_events(&events, &id_to_name, &sim);
    print_entities(&sim, &id_to_name);

    // Entity deviations since end of Round 1.
    let deviations_r2 = entity_deviations_since(&*sim.world(), post_r1_batch);
    let notable: Vec<_> = deviations_r2.iter()
        .filter(|d| d.coherence_delta.abs() > 0.05 || d.membership_event_count > 0 || d.went_dormant || d.born_after_baseline)
        .collect();
    if !notable.is_empty() {
        println!("  [deviations since Round 1]");
        for d in notable.iter().take(5) {
            let status = if d.born_after_baseline { "NEW" }
                else if d.went_dormant { "DORMANT" }
                else { "changed" };
            println!("    entity#{:<3} {:8}  Δcoherence={:+.3}  Δmembers={:+}",
                d.entity_id.0, status, d.coherence_delta, d.member_count_delta);
        }
    }

    // Counterfactual for nociception.
    let absent_r2 = relationships_absent_without(&*sim.world(), &r2_roots);
    println!("  [counterfactual] {} relationship(s) born from nociception stimulus", absent_r2.len());

    // ── Round 3: Chemosensory (olfaction) ────────────────────────────────
    println!("\n--- Round 3: Olfaction (AWC, AWA) ---");
    let pre_r3_batch = sim.world().current_batch();
    for _ in 0..5 {
        stimulate(&mut sim, &name_to_id, &["AWCL", "AWCR", "AWAL", "AWAR"], 0.6);
    }
    let post_r3_batch = sim.world().current_batch();
    let r3_roots: Vec<_> = sim.world().log()
        .batch(pre_r3_batch)
        .map(|c| c.id)
        .collect();
    let events = sim.recognize_entities(&ep);
    print_emergence_events(&events, &id_to_name, &sim);
    print_entities(&sim, &id_to_name);

    // Entity deviations since end of Round 2.
    let deviations_r3 = entity_deviations_since(&*sim.world(), post_r2_batch);
    let notable: Vec<_> = deviations_r3.iter()
        .filter(|d| d.coherence_delta.abs() > 0.05 || d.membership_event_count > 0 || d.went_dormant || d.born_after_baseline)
        .collect();
    if !notable.is_empty() {
        println!("  [deviations since Round 2]");
        for d in notable.iter().take(5) {
            let status = if d.born_after_baseline { "NEW" }
                else if d.went_dormant { "DORMANT" }
                else { "changed" };
            println!("    entity#{:<3} {:8}  Δcoherence={:+.3}  Δmembers={:+}",
                d.entity_id.0, status, d.coherence_delta, d.member_count_delta);
        }
    }

    let absent_r3 = relationships_absent_without(&*sim.world(), &r3_roots);
    println!("  [counterfactual] {} relationship(s) born from olfaction stimulus", absent_r3.len());

    // ── Cohere clusters ───────────────────────────────────────────────────
    println!("\n--- Cohere clusters ---");
    sim.extract_cohere(&cp);
    let coheres_guard = sim.world();
    let coheres = coheres_guard.coheres().get("default").unwrap_or(&[]);
    println!("  {} cluster(s) found", coheres.len());
    for c in coheres {
        println!("  cohere#{}  strength={:.3}", c.id.0, c.strength);
    }

    // ── Entity summaries (named, sorted by coherence) ─────────────────────
    println!("\n--- Entity summaries ---");
    let summaries = entities_summary(&*sim.world(), &names);
    for s in summaries.iter().take(8) {
        println!("  {s}");
    }
    if summaries.len() > 8 {
        println!("  … and {} more entities", summaries.len() - 8);
    }

    // ── Top active relationships (named) ──────────────────────────────────
    println!("\n--- Top 10 most active synapses ---");
    let world_for_rels = sim.world();
    let mut all_rels: Vec<_> = world_for_rels.relationships().iter().collect();
    all_rels.sort_by(|a, b| b.activity().partial_cmp(&a.activity()).unwrap_or(std::cmp::Ordering::Equal));
    let top10: Vec<_> = all_rels.iter().take(10).map(|r| r.id).collect();
    // Build a temporary filtered list using relationship_list then pick top 10.
    for rel in &all_rels[..10.min(all_rels.len())] {
        let (from, to) = match rel.endpoints {
            Endpoints::Directed { from, to } => (from, to),
            Endpoints::Symmetric { a, b } => (a, b),
        };
        let kind_str = if rel.kind == KIND_EXC { "exc" } else { "inh" };
        println!(
            "  {:8} →{:4}→ {:8}  activity={:.3}  weight={:.4}  touches={}",
            names.name(from), kind_str, names.name(to),
            rel.activity(), rel.weight(), rel.lineage.change_count
        );
    }

    // ── DOT export ────────────────────────────────────────────────────────
    let dot = to_dot_named(&*sim.world(), &names);
    let dot_path = "celegans_graph.dot";
    match std::fs::write(dot_path, &dot) {
        Ok(_) => println!("\n  DOT graph written to {dot_path}  ({} bytes)", dot.len()),
        Err(e) => println!("\n  (DOT export failed: {e})"),
    }

    // ── Summary ──────────────────────────────────────────────────────────
    println!("\n--- Summary ---");
    {
        let w = sim.world();
        println!("  Relationships emerged: {}", w.relationships().len());
        println!("  Entities active:       {}", w.entities().active_count());
        println!("  Changes committed:     {}", w.log().len());
        println!("  Current batch:         {:?}", w.current_batch());
    }

    // suppress unused warnings on kept variables
    let _ = (pre_r1_batch, post_r3_batch, top10, id_to_name);
    let _ = r3_roots;
}

// ── Print helpers ─────────────────────────────────────────────────────────────

fn print_emergence_events(
    events: &[graph_core::WorldEvent],
    _id_to_name: &HashMap<LocusId, String>,
    _sim: &graph_engine::Simulation,
) {
    use graph_core::WorldEvent;
    for ev in events {
        match ev {
            WorldEvent::EntityBorn { entity, batch, member_count } => {
                println!("  [BORN]    entity#{} batch={:?} members={}", entity.0, batch, member_count);
            }
            WorldEvent::EntityDormant { entity, batch } => {
                println!("  [DORMANT] entity#{} batch={:?}", entity.0, batch);
            }
            WorldEvent::EntityRevived { entity, batch } => {
                println!("  [REVIVED] entity#{} batch={:?}", entity.0, batch);
            }
            WorldEvent::CoherenceShift { entity, from, to, .. } => {
                println!("  [SHIFT]   entity#{} coherence {:.3}→{:.3}", entity.0, from, to);
            }
            _ => {}
        }
    }
}

fn print_entities(
    sim: &graph_engine::Simulation,
    id_to_name: &HashMap<LocusId, String>,
) {
    let world_guard = sim.world();
    let entities: Vec<_> = world_guard.entities().active().collect();
    if entities.is_empty() {
        println!("  (no active entities)");
        return;
    }
    for e in &entities {
        // Map member IDs to names; truncate display at 8.
        let mut member_names: Vec<&str> = e.current.members.iter()
            .filter_map(|id| id_to_name.get(id).map(String::as_str))
            .collect();
        member_names.sort();
        let display: Vec<&str> = member_names.iter().copied().take(8).collect();
        let suffix = if member_names.len() > 8 {
            format!(" …+{}", member_names.len() - 8)
        } else {
            String::new()
        };
        println!(
            "  entity#{:<3} members={:<3} coherence={:.3}  [{}{}]",
            e.id.0,
            e.current.members.len(),
            e.current.coherence,
            display.join(", "),
            suffix,
        );
    }
}
