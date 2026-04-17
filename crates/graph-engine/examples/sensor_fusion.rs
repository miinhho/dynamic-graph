//! Sensor-fusion HVAC example.
//!
//! A non-trivial dogfood that exercises three features added in the
//! current redesign iteration:
//!
//! 1. **`LocusContext` queries** (O2): the aggregator reads ALL sensor
//!    states from context — not just the subset that fired this batch —
//!    so it always produces a complete zone average even when only one
//!    sensor changed.
//!
//! 2. **Structural proposals**: when a sensor reading exceeds a bypass
//!    threshold, the sensor proposes a direct link to the controller,
//!    short-circuiting the aggregator for high-priority alerts.  After
//!    the bypass is created, the controller receives both the aggregated
//!    average (from AGG) and the raw spike (direct from S1) in
//!    subsequent ticks.
//!
//! 3. **Hebbian plasticity**: frequently co-activated sensor→aggregator
//!    links accumulate weight, letting the relationship store reflect
//!    channel reliability over time.
//!
//! Topology:
//!
//! ```text
//!   S1 ──→ AGG ──→ CTRL
//!   S2 ──↗
//!   S3 ──↗
//!
//!   (after heat spike)
//!   S1 ──→ CTRL   (bypass, direct)
//! ```
//!
//! Run: `cargo run -p graph-engine --example sensor_fusion`

use graph_core::{
    Change, ChangeSubject, Endpoints, InfluenceKindId, Locus, LocusContext, LocusId, LocusKindId,
    LocusProgram, ProposedChange, StateVector, StructuralProposal,
};
use graph_engine::{
    DefaultCoherePerspective, DefaultEmergencePerspective, Engine, EngineConfig,
    InfluenceKindConfig, InfluenceKindRegistry, LocusKindRegistry, PlasticityConfig,
};
use graph_query::{connected_components, path_between};
use graph_world::World;

// ── Kind constants ────────────────────────────────────────────────────────────

const KIND_SENSOR: LocusKindId = LocusKindId(1);
const KIND_AGGREGATOR: LocusKindId = LocusKindId(2);
const KIND_CONTROLLER: LocusKindId = LocusKindId(3);

/// Temperature readings — Hebbian plasticity enabled.
const KIND_TEMP: InfluenceKindId = InfluenceKindId(1);
/// Controller correction output — stored on CTRL itself.
const KIND_CTRL: InfluenceKindId = InfluenceKindId(2);

// ── Locus IDs ─────────────────────────────────────────────────────────────────

const S1: LocusId = LocusId(1);
const S2: LocusId = LocusId(2);
const S3: LocusId = LocusId(3);
const AGG: LocusId = LocusId(4);
const CTRL: LocusId = LocusId(5);

// ── Programs ──────────────────────────────────────────────────────────────────

/// Temperature sensor — one program instance shared by all three sensors.
///
/// On a KIND_TEMP stimulus: forward the reading to the aggregator.
///
/// In `structural_proposals`: if the committed locus state exceeds
/// `bypass_threshold`, propose a direct Directed relationship from this
/// sensor to the controller.  This fires after the commit so `locus.state`
/// already holds the new temperature value.
struct SensorProgram {
    aggregator: LocusId,
    controller: LocusId,
    bypass_threshold: f32,
}

impl LocusProgram for SensorProgram {
    fn process(
        &self,
        locus: &Locus,
        incoming: &[&Change],
        ctx: &dyn LocusContext,
    ) -> Vec<ProposedChange> {
        // Only forward direct stimuli (no causal predecessors).
        let reading: f32 = incoming
            .iter()
            .filter(|c| c.is_stimulus())
            .flat_map(|c| c.after.as_slice())
            .copied()
            .sum();
        if reading < 1e-6 {
            return vec![];
        }
        let mut proposals = vec![ProposedChange::new(
            ChangeSubject::Locus(self.aggregator),
            KIND_TEMP,
            StateVector::from_slice(&[reading]),
        )];
        // If a prior structural proposal created a bypass link from this
        // sensor directly to the controller, forward the raw reading too.
        if ctx.relationship_between(locus.id, self.controller).is_some() {
            proposals.push(ProposedChange::new(
                ChangeSubject::Locus(self.controller),
                KIND_TEMP,
                StateVector::from_slice(&[reading]),
            ));
        }
        proposals
    }

    fn structural_proposals(
        &self,
        locus: &Locus,
        _incoming: &[&Change],
        _ctx: &dyn LocusContext,
    ) -> Vec<StructuralProposal> {
        let temp = locus.state.as_slice().first().copied().unwrap_or(0.0);
        if temp > self.bypass_threshold {
            return vec![StructuralProposal::CreateRelationship {
                endpoints: Endpoints::Directed { from: locus.id, to: self.controller },
                kind: KIND_TEMP,
                initial_activity: None,
                initial_state: None,
            }];
        }
        vec![]
    }
}

/// Zone aggregator.
///
/// Triggered when any sensor fires into it.  Uses `LocusContext` to read
/// the current state of ALL sensors — including those that did not change
/// this batch — to compute a complete zone average rather than a partial one.
struct AggregatorProgram {
    sensor_ids: Vec<LocusId>,
    controller: LocusId,
}

impl LocusProgram for AggregatorProgram {
    fn process(
        &self,
        _locus: &Locus,
        incoming: &[&Change],
        ctx: &dyn LocusContext,
    ) -> Vec<ProposedChange> {
        if incoming.is_empty() {
            return vec![];
        }
        // Sensors that haven't been stimulated yet have state 0 — exclude
        // them so they don't drag the average down artificially.
        let (sum, count) = self
            .sensor_ids
            .iter()
            .filter_map(|&sid| ctx.locus(sid))
            .filter_map(|l| l.state.as_slice().first().copied())
            .filter(|&v| v > 1e-6)
            .fold((0.0f32, 0usize), |(s, n), v| (s + v, n + 1));

        if count == 0 {
            return vec![];
        }
        let avg = sum / count as f32;
        vec![ProposedChange::new(
            ChangeSubject::Locus(self.controller),
            KIND_TEMP,
            StateVector::from_slice(&[avg]),
        )]
    }
}

/// HVAC controller.
///
/// Receives aggregated temperatures (and, after the bypass is created, a
/// direct reading from S1 as well).  Uses `LocusContext` to identify the
/// hottest individual sensor for diagnostic logging.  Stores a correction
/// signal on itself; filters its own feedback so it does not loop.
struct ControllerProgram {
    sensor_ids: Vec<LocusId>,
    setpoint: f32,
}

impl LocusProgram for ControllerProgram {
    fn process(
        &self,
        locus: &Locus,
        incoming: &[&Change],
        ctx: &dyn LocusContext,
    ) -> Vec<ProposedChange> {
        // Ignore our own correction feedback — only react to temperature changes.
        let (sum, count) = incoming
            .iter()
            .filter(|c| c.kind == KIND_TEMP)
            .flat_map(|c| c.after.as_slice())
            .copied()
            .fold((0.0f32, 0usize), |(s, n), v| (s + v, n + 1));
        if count == 0 {
            return vec![];
        }
        let avg_temp = sum / count as f32;

        // Use LocusContext to find the hottest individual sensor.
        let hottest = self
            .sensor_ids
            .iter()
            .filter_map(|&sid| {
                ctx.locus(sid).and_then(|l| {
                    l.state.as_slice().first().copied().map(|v| (sid, v))
                })
            })
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

        if let Some((hot_id, hot_temp)) = hottest
            && hot_temp > 0.01
        {
            println!(
                "    [CTRL ctx-query] hottest=S{}({:.3})  avg_in={:.3}  correction={:+.3}",
                hot_id.0,
                hot_temp,
                avg_temp,
                self.setpoint - avg_temp
            );
        }

        let correction = (self.setpoint - avg_temp).clamp(-1.0, 1.0);
        vec![ProposedChange::new(
            ChangeSubject::Locus(locus.id),
            KIND_CTRL,
            StateVector::from_slice(&[correction]),
        )]
    }
}

// ── World construction ────────────────────────────────────────────────────────

fn build_world() -> (World, LocusKindRegistry, InfluenceKindRegistry) {
    let mut world = World::new();
    for sid in [S1, S2, S3] {
        world.insert_locus(Locus::new(sid, KIND_SENSOR, StateVector::zeros(1)));
    }
    world.insert_locus(Locus::new(AGG, KIND_AGGREGATOR, StateVector::zeros(1)));
    world.insert_locus(Locus::new(CTRL, KIND_CONTROLLER, StateVector::zeros(1)));

    let mut loci = LocusKindRegistry::new();
    loci.insert(
        KIND_SENSOR,
        Box::new(SensorProgram {
            aggregator: AGG,
            controller: CTRL,
            bypass_threshold: 0.75,
        }),
    );
    loci.insert(
        KIND_AGGREGATOR,
        Box::new(AggregatorProgram { sensor_ids: vec![S1, S2, S3], controller: CTRL }),
    );
    loci.insert(
        KIND_CONTROLLER,
        Box::new(ControllerProgram {
            sensor_ids: vec![S1, S2, S3],
            setpoint: 0.50,
        }),
    );

    let mut influences = InfluenceKindRegistry::new();
    influences.insert(
        KIND_TEMP,
        InfluenceKindConfig::new("temperature")
            .with_decay(0.85)
            .with_plasticity(PlasticityConfig {
                learning_rate: 0.05,
                weight_decay: 0.99,
                max_weight: 2.0,
                stdp: false,
            ..Default::default()
            }),
    );
    influences.insert(
        KIND_CTRL,
        InfluenceKindConfig::new("control-correction").with_decay(1.0),
    );

    (world, loci, influences)
}

fn stimulus(locus: LocusId, temp: f32) -> ProposedChange {
    ProposedChange::stimulus(locus, KIND_TEMP, &[temp])
}

// ── Main ──────────────────────────────────────────────────────────────────────

fn main() {
    let (mut world, loci, influences) = build_world();
    let engine = Engine::new(EngineConfig {
        max_batches_per_tick: 16,
    });

    println!("=== Sensor Fusion HVAC Example ===\n");
    println!("  S1, S2, S3 → AGG → CTRL");
    println!("  bypass threshold: 0.75  setpoint: 0.50\n");

    // ── Tick 1: all sensors near setpoint ─────────────────────────────────────

    println!("--- Tick 1: normal readings (S1=0.45, S2=0.50, S3=0.55) ---");
    let r1 = engine.tick(
        &mut world,
        &loci,
        &influences,
        vec![stimulus(S1, 0.45), stimulus(S2, 0.50), stimulus(S3, 0.55)],
    );
    println!(
        "  batches={} changes={} relationships={}",
        r1.batches_committed,
        r1.changes_committed,
        world.relationships().len()
    );
    println!();

    // ── Tick 2: heat spike on S1 — triggers bypass proposal ───────────────────

    println!("--- Tick 2: heat spike (S1=0.90, S2=0.50, S3=0.45) ---");
    let r2 = engine.tick(
        &mut world,
        &loci,
        &influences,
        vec![stimulus(S1, 0.90), stimulus(S2, 0.50), stimulus(S3, 0.45)],
    );
    println!(
        "  batches={} changes={} relationships={}",
        r2.batches_committed,
        r2.changes_committed,
        world.relationships().len()
    );
    let has_bypass = world.relationships().iter().any(|r| {
        matches!(&r.endpoints, Endpoints::Directed { from, to } if *from == S1 && *to == CTRL)
    });
    println!("  S1 → CTRL bypass created: {has_bypass}");
    println!();

    // ── Tick 3: S1 alone fires — both S1→AGG→CTRL and S1→CTRL paths active ───

    println!("--- Tick 3: S1 alone (0.82) — bypass exercises direct path ---");
    let r3 = engine.tick(
        &mut world,
        &loci,
        &influences,
        vec![stimulus(S1, 0.82)],
    );
    println!(
        "  batches={} changes={} relationships={}",
        r3.batches_committed,
        r3.changes_committed,
        world.relationships().len()
    );
    println!();

    // ── Tick 4: all sensors nominal — system returns toward setpoint ──────────

    println!("--- Tick 4: return to normal (S1=0.48, S2=0.51, S3=0.52) ---");
    let r4 = engine.tick(
        &mut world,
        &loci,
        &influences,
        vec![stimulus(S1, 0.48), stimulus(S2, 0.51), stimulus(S3, 0.52)],
    );
    println!(
        "  batches={} changes={} relationships={}",
        r4.batches_committed,
        r4.changes_committed,
        world.relationships().len()
    );
    println!();

    // ── Relationship snapshot ──────────────────────────────────────────────────

    engine.flush_relationship_decay(&mut world, &influences);
    println!("--- Relationships (after flush) ---");
    for r in world.relationships().iter() {
        let (f, t) = match &r.endpoints {
            Endpoints::Directed { from, to } => (from.0, to.0),
            _ => (0, 0),
        };
        let tag = if f == S1.0 && t == CTRL.0 { " ← bypass" } else { "" };
        println!(
            "  L{}→L{}  activity={:.4}  weight={:.4}  touches={}{}",
            f, t, r.activity(), r.weight(), r.lineage.change_count, tag
        );
    }
    println!();

    // ── Graph traversal ────────────────────────────────────────────────────────

    match path_between(&world, S1, CTRL).as_deref() {
        Some([_, _]) => println!("Shortest path S1 → CTRL: direct (bypass active, 1 hop)"),
        Some(p) => {
            let hops: Vec<_> = p.iter().map(|l| format!("L{}", l.0)).collect();
            println!("Shortest path S1 → CTRL: {} ({} hops)", hops.join(" → "), p.len() - 1);
        }
        None => println!("Shortest path S1 → CTRL: no path"),
    }

    let components = connected_components(&world);
    println!(
        "Connected components: {} (fully connected: {})",
        components.len(),
        components.len() == 1
    );
    println!();

    // ── Entity recognition ─────────────────────────────────────────────────────

    let ep = DefaultEmergencePerspective {
        min_activity_threshold: 0.01,
        ..Default::default()
    };
    engine.recognize_entities(&mut world, &influences, &ep);

    println!(
        "--- Entities ({} active) ---",
        world.entities().active_count()
    );
    for e in world.entities().active() {
        let members: Vec<u64> = e.current.members.iter().map(|l| l.0).collect();
        println!(
            "  entity#{} members={members:?} coherence={:.3} layers={}",
            e.id.0, e.current.coherence, e.layer_count()
        );
    }
    println!();

    // ── Cohere extraction ──────────────────────────────────────────────────────

    let cp = DefaultCoherePerspective {
        min_bridge_activity: 0.01,
        ..Default::default()
    };
    engine.extract_cohere(&mut world, &influences, &cp);

    let coheres = world.coheres().get("default").unwrap_or(&[]);
    println!("--- Coheres ({}) ---", coheres.len());
    for c in coheres {
        let ms = match &c.members {
            graph_core::CohereMembers::Entities(ids) => ids
                .iter()
                .map(|e| format!("entity#{}", e.0))
                .collect::<Vec<_>>()
                .join(", "),
            _ => "(mixed)".to_string(),
        };
        println!("  cohere#{} [{ms}] strength={:.3}", c.id.0, c.strength);
    }
    if coheres.is_empty() {
        println!("  (none — entities not bridged above threshold)");
    }

    // ── Plasticity summary ─────────────────────────────────────────────────────
    // After 4 ticks the weights are small but non-zero — each co-activated
    // step accumulates Δweight = lr(0.05) × pre × post.  The AGG→CTRL link
    // leads because it is touched in every tick; the bypass S1→CTRL lags
    // because it only existed from Tick 2 onwards.

    println!();
    println!("--- Hebbian weight accumulation (lr=0.05, decay=0.99/batch) ---");
    let mut rels: Vec<_> = world.relationships().iter().collect();
    rels.sort_by(|a, b| b.weight().partial_cmp(&a.weight()).unwrap_or(std::cmp::Ordering::Equal));
    for r in rels {
        let (f, t) = match &r.endpoints {
            Endpoints::Directed { from, to } => (from.0, to.0),
            _ => (0, 0),
        };
        println!("  L{}→L{}  weight={:.4}  touches={}", f, t, r.weight(), r.lineage.change_count);
    }

    println!("\nDone.");
}
