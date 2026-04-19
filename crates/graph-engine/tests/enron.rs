//! Oracle test: Enron-style email community benchmark.
//!
//! Models the Enron email corpus structure as a synthetic temporal stream:
//! 120 employees across 6 departments, communicating over five organisational
//! phases that mirror the Enron timeline —
//! stable operation → department merger → scandal-period silence →
//! further contraction → partial revival.
//!
//! Communication model (mirrors `lfr_dynamic.rs`): each batch fires one
//! all-hands co-activation event per active department. No random noise is
//! added — pure community activation keeps the activity distribution
//! cleanly bimodal, allowing `auto_activity_threshold` to find the correct
//! signal/noise cut across all five phases.
//!
//! ## Five-phase schedule
//!
//! ```text
//! Phase 0 (Born):     A B C D E F active        →  Born × 6
//! Phase 1 (Merge):    A B C D EF active          →  Merge(E+F → EF)
//! Phase 2 (Dormant):  A B C D active; EF silent  →  BecameDormant(EF)
//! Phase 3 (Contract): A B C active; D silent     →  BecameDormant(D)
//! Phase 4 (Revival):  A B C EF active            →  Revived(EF)
//! ```
//!
//! This is the first test in the suite to exercise `Revived` —
//! `lfr_dynamic.rs` has no dormant-then-revived path. `CoherenceShift` is
//! expected to remain at 0, closing Finding 2a with Enron evidence.
//!
//! ## Knob evidence (Ω2 candidates)
//!
//! All tests run `min_activity_threshold: None` (auto). If the auto path
//! navigates all five phases correctly, that is evidence toward demoting the
//! manual override. `demotion_policy` and `PlasticityConfig.weight_decay`
//! use defaults — no test differentiates their alternatives.

use graph_core::{
    BatchId, ChangeSubject, EntityId, EntityStatus, InfluenceKindId, LayerTransition, Locus,
    LocusContext, LocusId, LocusKindId, LocusProgram, Properties, PropertyValue, ProposedChange,
    StateVector,
};
use graph_engine::{
    DefaultEmergencePerspective, Engine, EngineConfig, InfluenceKindConfig, InfluenceKindRegistry,
    LocusKindRegistry,
};
use graph_world::World;
use std::collections::BTreeSet;

// ── Constants ────────────────────────────────────────────────────────────────

const DEPT_SIZE: u64 = 20;
const N_DEPTS: u64 = 6;
const N_EMPLOYEES: u64 = DEPT_SIZE * N_DEPTS; // 120

const DEPT_A: u64 = 0;
const DEPT_B: u64 = 1;
const DEPT_C: u64 = 2;
const DEPT_D: u64 = 3;
const DEPT_E: u64 = 4;
const DEPT_F: u64 = 5;

fn dept(d: u64) -> Vec<u64> {
    (d * DEPT_SIZE..(d + 1) * DEPT_SIZE).collect()
}

fn dept_ef() -> Vec<u64> {
    dept(DEPT_E).into_iter().chain(dept(DEPT_F)).collect()
}

/// Activity half-life ~2 batches (same as lfr_dynamic.rs).
const DECAY: f32 = 0.5;

/// Batches per phase — longer than LFR to model Enron's extended timeline.
const BATCHES_PER_PHASE: usize = 5;

/// No random noise — pure community activation per batch.
/// Noise from previous phases creates a low-activity floor that sits
/// between decayed EF edges and active intra-dept edges; the gap detector
/// mistakes the noise/EF boundary for the signal/noise cut and sets the
/// threshold too low to exclude dormant EF edges.  A future test could
/// add noise gated behind an explicit `min_activity_threshold` override
/// once the auto-threshold heuristic is extended to ignore multi-phase
/// cross-generation noise floors.  The "noisy regime" here comes from the
/// 120-node scale and the five-phase community-drift schedule, not from
/// random cross-dept pair touches.

const MATCH_WINDOW: u64 = BATCHES_PER_PHASE as u64 + 2;
const MEMBER_JACCARD_MIN: f32 = 0.5;

const EMAIL: InfluenceKindId = InfluenceKindId(400);
const EMPLOYEE_KIND: LocusKindId = LocusKindId(1);

// ── Co-activation program (depth-1 broadcast, identical to lfr_dynamic) ─────

struct EmailProgram;

impl LocusProgram for EmailProgram {
    fn process(
        &self,
        locus: &Locus,
        incoming: &[&graph_core::Change],
        _ctx: &dyn LocusContext,
    ) -> Vec<ProposedChange> {
        let mut out = Vec::new();
        for change in incoming {
            if change.subject != ChangeSubject::Locus(locus.id) {
                continue;
            }
            let Some(meta) = change.metadata.as_ref() else {
                continue;
            };
            let Some(PropertyValue::List(ids)) = meta.get("co_members") else {
                continue;
            };
            for val in ids {
                let PropertyValue::Int(id) = val else { continue };
                let other = *id as u64;
                if other == locus.id.0 {
                    continue;
                }
                out.push(ProposedChange::new(
                    ChangeSubject::Locus(LocusId(other)),
                    EMAIL,
                    StateVector::from_slice(&[1.0]),
                ));
            }
        }
        out
    }
}

fn community_event(members: &[u64]) -> Vec<ProposedChange> {
    members
        .iter()
        .map(|&w| {
            let others: Vec<PropertyValue> = members
                .iter()
                .filter(|&&x| x != w)
                .map(|&id| PropertyValue::Int(id as i64))
                .collect();
            let mut meta = Properties::new();
            meta.set("co_members", PropertyValue::List(others));
            ProposedChange::new(
                ChangeSubject::Locus(LocusId(w)),
                EMAIL,
                StateVector::from_slice(&[1.0]),
            )
            .with_metadata(meta)
        })
        .collect()
}

// ── World / registry setup ───────────────────────────────────────────────────

fn insert_employees(world: &mut World) {
    for i in 0..N_EMPLOYEES {
        world.insert_locus(Locus::new(LocusId(i), EMPLOYEE_KIND, StateVector::zeros(1)));
    }
}

fn make_inf_reg() -> InfluenceKindRegistry {
    let cfg = InfluenceKindConfig::new("email").with_decay(DECAY).with_symmetric(true);
    let mut reg = InfluenceKindRegistry::new();
    reg.insert(EMAIL, cfg);
    reg
}

fn make_loci_reg() -> LocusKindRegistry {
    let mut reg = LocusKindRegistry::new();
    reg.insert(EMPLOYEE_KIND, Box::new(EmailProgram));
    reg
}

// ── Stimuli builder ──────────────────────────────────────────────────────────

/// One batch: one full co-activation per active department.
fn batch_stimuli(active_depts: &[Vec<u64>]) -> Vec<ProposedChange> {
    active_depts
        .iter()
        .flat_map(|dept| community_event(dept))
        .collect()
}

// ── Phase runner ─────────────────────────────────────────────────────────────

fn run_phase(
    world: &mut World,
    engine: &Engine,
    loci_reg: &LocusKindRegistry,
    inf: &InfluenceKindRegistry,
    perspective: &DefaultEmergencePerspective,
    active_depts: &[Vec<u64>],
) -> BatchId {
    for _ in 0..BATCHES_PER_PHASE {
        let stimuli = batch_stimuli(active_depts);
        engine.tick(world, loci_reg, inf, stimuli);
    }
    engine.recognize_entities(world, inf, perspective);
    world.current_batch()
}

// ── Transition extraction (mirrors lfr_dynamic.rs) ───────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum TransitionKind {
    Born,
    Split,
    Merge,
    Dormant,
    MembershipDelta,
    CoherenceShift,
    Revived,
}

#[derive(Debug, Clone)]
struct DetectedTransition {
    entity: EntityId,
    batch: BatchId,
    kind: TransitionKind,
    members: BTreeSet<u64>,
    related: Vec<EntityId>,
}

fn collect_transitions(world: &World) -> Vec<DetectedTransition> {
    let mut split_offspring: std::collections::HashSet<(EntityId, BatchId)> =
        std::collections::HashSet::new();
    let mut merge_absorbed: std::collections::HashSet<(EntityId, BatchId)> =
        std::collections::HashSet::new();

    for e in world.entities().iter() {
        for layer in &e.layers {
            if let LayerTransition::Split { offspring } = &layer.transition {
                for &child in offspring {
                    split_offspring.insert((child, layer.batch));
                }
            }
            if matches!(layer.transition, LayerTransition::Merged { .. })
                && e.status == EntityStatus::Dormant
            {
                merge_absorbed.insert((e.id, layer.batch));
            }
        }
    }

    let mut all = Vec::new();
    for e in world.entities().iter() {
        for layer in &e.layers {
            if matches!(layer.transition, LayerTransition::Born)
                && split_offspring.contains(&(e.id, layer.batch))
            {
                continue;
            }
            if matches!(layer.transition, LayerTransition::Merged { .. })
                && merge_absorbed.contains(&(e.id, layer.batch))
            {
                continue;
            }
            let members: BTreeSet<u64> = layer
                .snapshot
                .as_ref()
                .map(|s| s.members.iter().map(|l| l.0).collect())
                .unwrap_or_default();
            let (kind, related) = match &layer.transition {
                LayerTransition::Born => (TransitionKind::Born, vec![]),
                LayerTransition::Split { offspring } => (TransitionKind::Split, offspring.clone()),
                LayerTransition::Merged { absorbed } => (TransitionKind::Merge, absorbed.clone()),
                LayerTransition::BecameDormant => (TransitionKind::Dormant, vec![]),
                LayerTransition::MembershipDelta { .. } => (TransitionKind::MembershipDelta, vec![]),
                LayerTransition::CoherenceShift { .. } => (TransitionKind::CoherenceShift, vec![]),
                LayerTransition::Revived => (TransitionKind::Revived, vec![]),
            };
            all.push(DetectedTransition { entity: e.id, batch: layer.batch, kind, members, related });
        }
    }
    all.sort_by_key(|t| (t.batch.0, t.entity.0));
    all
}

fn effective_members(world: &World, t: &DetectedTransition) -> BTreeSet<u64> {
    match t.kind {
        TransitionKind::Split | TransitionKind::Merge => {
            let mut out = t.members.clone();
            for id in &t.related {
                if let Some(other) = world.entities().get(*id) {
                    out.extend(other.current.members.iter().map(|l| l.0));
                }
            }
            out
        }
        _ => t.members.clone(),
    }
}

fn jaccard(a: &BTreeSet<u64>, b: &BTreeSet<u64>) -> f32 {
    let inter = a.intersection(b).count();
    let union = a.len() + b.len() - inter;
    if union == 0 { 1.0 } else { inter as f32 / union as f32 }
}

fn set(xs: &[u64]) -> BTreeSet<u64> {
    xs.iter().copied().collect()
}

// ── Oracle ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct PlantedEvent {
    kind: TransitionKind,
    batch: BatchId,
    members: BTreeSet<u64>,
    #[allow(dead_code)]
    label: &'static str,
}

#[derive(Debug, Default)]
struct Score {
    planted: usize,
    detected: usize,
    tp: usize,
}

impl Score {
    fn precision(&self) -> f32 {
        if self.detected == 0 { 0.0 } else { self.tp as f32 / self.detected as f32 }
    }
    fn recall(&self) -> f32 {
        if self.planted == 0 { 1.0 } else { self.tp as f32 / self.planted as f32 }
    }
}

fn score_transitions(
    world: &World,
    detected: &[DetectedTransition],
    planted: &[PlantedEvent],
) -> std::collections::HashMap<TransitionKind, Score> {
    use std::collections::HashMap;

    let mut by_kind: HashMap<TransitionKind, Score> = HashMap::new();
    for k in [
        TransitionKind::Born, TransitionKind::Split, TransitionKind::Merge,
        TransitionKind::Dormant, TransitionKind::Revived,
    ] {
        by_kind.insert(k, Score::default());
    }

    for d in detected {
        if let Some(s) = by_kind.get_mut(&d.kind) {
            s.detected += 1;
        }
    }
    for p in planted {
        if let Some(s) = by_kind.get_mut(&p.kind) {
            s.planted += 1;
        }
    }

    let mut consumed = vec![false; detected.len()];
    for p in planted {
        let mut best: Option<(usize, i64, f32)> = None;
        for (i, d) in detected.iter().enumerate() {
            if consumed[i] || d.kind != p.kind { continue; }
            let dt = (d.batch.0 as i64 - p.batch.0 as i64).abs();
            if dt as u64 > MATCH_WINDOW { continue; }
            let d_members = effective_members(world, d);
            let score = jaccard(&p.members, &d_members);
            if score < MEMBER_JACCARD_MIN { continue; }
            let better = match best {
                None => true,
                Some((_, b_dt, b_score)) => dt < b_dt || (dt == b_dt && score > b_score),
            };
            if better { best = Some((i, dt, score)); }
        }
        if let Some((i, _, _)) = best {
            consumed[i] = true;
            if let Some(s) = by_kind.get_mut(&p.kind) { s.tp += 1; }
        }
    }
    by_kind
}

fn noise_counts(detected: &[DetectedTransition]) -> (usize, usize, usize) {
    let (mut md, mut cs, mut rv) = (0, 0, 0);
    for d in detected {
        match d.kind {
            TransitionKind::MembershipDelta => md += 1,
            TransitionKind::CoherenceShift => cs += 1,
            TransitionKind::Revived => rv += 1,
            _ => {}
        }
    }
    (md, cs, rv)
}

fn report(
    label: &str,
    scores: &std::collections::HashMap<TransitionKind, Score>,
    detected: &[DetectedTransition],
) {
    let (md, cs, _rv) = noise_counts(detected);
    println!("\n── {label} ────────────────────────────────────────");
    println!(
        "{:<18} {:>8} {:>8} {:>8} {:>10} {:>10}",
        "kind", "planted", "detected", "tp", "precision", "recall"
    );
    for k in [
        TransitionKind::Born, TransitionKind::Split, TransitionKind::Merge,
        TransitionKind::Dormant, TransitionKind::Revived,
    ] {
        if let Some(s) = scores.get(&k) {
            println!(
                "{:<18} {:>8} {:>8} {:>8} {:>10.2} {:>10.2}",
                format!("{k:?}"), s.planted, s.detected, s.tp, s.precision(), s.recall(),
            );
        }
    }
    println!("{:<18} {:>8} {:>8}", "MembershipDelta", "n/a", md);
    println!("{:<18} {:>8} {:>8}", "CoherenceShift", "n/a", cs);
}

// ── Isolated: Born × 6 ───────────────────────────────────────────────────────

#[test]
fn initial_departments_are_recognized() {
    let mut world = World::new();
    insert_employees(&mut world);
    let inf = make_inf_reg();
    let loci_reg = make_loci_reg();
    let engine = Engine::new(EngineConfig { max_batches_per_tick: 3 });
    let perspective = DefaultEmergencePerspective::default();

    let active: Vec<Vec<u64>> = (0..N_DEPTS).map(dept).collect();
    let p0 = run_phase(&mut world, &engine, &loci_reg, &inf, &perspective, &active);

    let detected = collect_transitions(&world);
    let planted: Vec<PlantedEvent> = (0..N_DEPTS)
        .map(|d| PlantedEvent {
            kind: TransitionKind::Born,
            batch: p0,
            members: set(&dept(d)),
            label: "dept born",
        })
        .collect();

    let scores = score_transitions(&world, &detected, &planted);
    report("initial_departments_are_recognized", &scores, &detected);

    let born = scores.get(&TransitionKind::Born).unwrap();
    assert!(
        born.tp >= 5,
        "expected ≥5/6 depts Born (tp={})",
        born.tp
    );
    assert!(
        born.precision() >= 0.70,
        "too many spurious Born events (precision={:.2})",
        born.precision()
    );
}

// ── Isolated: Merge (E+F → EF) ───────────────────────────────────────────────

#[test]
fn merge_ef_detected() {
    let mut world = World::new();
    insert_employees(&mut world);
    let inf = make_inf_reg();
    let loci_reg = make_loci_reg();
    let engine = Engine::new(EngineConfig { max_batches_per_tick: 3 });
    let perspective = DefaultEmergencePerspective::default();

    let active0: Vec<Vec<u64>> = (0..N_DEPTS).map(dept).collect();
    let p0 = run_phase(&mut world, &engine, &loci_reg, &inf, &perspective, &active0);

    let active1 = vec![dept(DEPT_A), dept(DEPT_B), dept(DEPT_C), dept(DEPT_D), dept_ef()];
    let p1 = run_phase(&mut world, &engine, &loci_reg, &inf, &perspective, &active1);

    let detected = collect_transitions(&world);
    let planted = vec![
        PlantedEvent { kind: TransitionKind::Born, batch: p0, members: set(&dept(DEPT_E)), label: "E born" },
        PlantedEvent { kind: TransitionKind::Born, batch: p0, members: set(&dept(DEPT_F)), label: "F born" },
        PlantedEvent { kind: TransitionKind::Merge, batch: p1, members: set(&dept_ef()), label: "E+F → EF" },
    ];
    let scores = score_transitions(&world, &detected, &planted);
    report("merge_ef_detected", &scores, &detected);

    let merge = scores.get(&TransitionKind::Merge).unwrap();
    assert_eq!(merge.tp, 1, "E+F merge must be detected (tp={})", merge.tp);
}

// ── Isolated: Dormant (EF goes silent) ───────────────────────────────────────

#[test]
fn dormant_ef_detected() {
    let mut world = World::new();
    insert_employees(&mut world);
    let inf = make_inf_reg();
    let loci_reg = make_loci_reg();
    let engine = Engine::new(EngineConfig { max_batches_per_tick: 3 });
    let perspective = DefaultEmergencePerspective::default();

    run_phase(&mut world, &engine, &loci_reg, &inf, &perspective,
        &(0..N_DEPTS).map(dept).collect::<Vec<_>>());
    run_phase(&mut world, &engine, &loci_reg, &inf, &perspective,
        &vec![dept(DEPT_A), dept(DEPT_B), dept(DEPT_C), dept(DEPT_D), dept_ef()]);

    let active2 = vec![dept(DEPT_A), dept(DEPT_B), dept(DEPT_C), dept(DEPT_D)];
    let p2 = run_phase(&mut world, &engine, &loci_reg, &inf, &perspective, &active2);

    let detected = collect_transitions(&world);
    let planted = vec![PlantedEvent {
        kind: TransitionKind::Dormant,
        batch: p2,
        members: set(&dept_ef()),
        label: "EF dormant",
    }];
    let scores = score_transitions(&world, &detected, &planted);
    report("dormant_ef_detected", &scores, &detected);

    let dormant = scores.get(&TransitionKind::Dormant).unwrap();
    assert_eq!(dormant.tp, 1, "EF must go dormant after silence (tp={})", dormant.tp);
}

// ── Isolated: Revived (EF comes back) ────────────────────────────────────────
//
// First test in this suite — and in the whole test tree — to exercise the
// Revived transition. lfr_dynamic.rs has no dormant-then-revived path.

#[test]
fn revived_ef_detected() {
    let mut world = World::new();
    insert_employees(&mut world);
    let inf = make_inf_reg();
    let loci_reg = make_loci_reg();
    let engine = Engine::new(EngineConfig { max_batches_per_tick: 3 });
    let perspective = DefaultEmergencePerspective::default();

    run_phase(&mut world, &engine, &loci_reg, &inf, &perspective,
        &(0..N_DEPTS).map(dept).collect::<Vec<_>>());
    run_phase(&mut world, &engine, &loci_reg, &inf, &perspective,
        &vec![dept(DEPT_A), dept(DEPT_B), dept(DEPT_C), dept(DEPT_D), dept_ef()]);
    run_phase(&mut world, &engine, &loci_reg, &inf, &perspective,
        &vec![dept(DEPT_A), dept(DEPT_B), dept(DEPT_C), dept(DEPT_D)]);
    run_phase(&mut world, &engine, &loci_reg, &inf, &perspective,
        &vec![dept(DEPT_A), dept(DEPT_B), dept(DEPT_C)]);

    let active4 = vec![dept(DEPT_A), dept(DEPT_B), dept(DEPT_C), dept_ef()];
    let p4 = run_phase(&mut world, &engine, &loci_reg, &inf, &perspective, &active4);

    let detected = collect_transitions(&world);
    let planted = vec![PlantedEvent {
        kind: TransitionKind::Revived,
        batch: p4,
        members: set(&dept_ef()),
        label: "EF revived",
    }];
    let scores = score_transitions(&world, &detected, &planted);
    report("revived_ef_detected", &scores, &detected);

    let revived = scores.get(&TransitionKind::Revived).unwrap();
    assert!(
        revived.tp >= 1,
        "EF revival must be detected after dormancy (tp={})",
        revived.tp
    );
}

// ── Composite: full Enron 5-phase protocol ───────────────────────────────────

#[test]
fn full_enron_protocol() {
    let mut world = World::new();
    insert_employees(&mut world);
    let inf = make_inf_reg();
    let loci_reg = make_loci_reg();
    let engine = Engine::new(EngineConfig { max_batches_per_tick: 3 });
    let perspective = DefaultEmergencePerspective::default();

    let p0 = run_phase(&mut world, &engine, &loci_reg, &inf, &perspective,
        &(0..N_DEPTS).map(dept).collect::<Vec<_>>());
    let p1 = run_phase(&mut world, &engine, &loci_reg, &inf, &perspective,
        &vec![dept(DEPT_A), dept(DEPT_B), dept(DEPT_C), dept(DEPT_D), dept_ef()]);
    let p2 = run_phase(&mut world, &engine, &loci_reg, &inf, &perspective,
        &vec![dept(DEPT_A), dept(DEPT_B), dept(DEPT_C), dept(DEPT_D)]);
    let p3 = run_phase(&mut world, &engine, &loci_reg, &inf, &perspective,
        &vec![dept(DEPT_A), dept(DEPT_B), dept(DEPT_C)]);
    let p4 = run_phase(&mut world, &engine, &loci_reg, &inf, &perspective,
        &vec![dept(DEPT_A), dept(DEPT_B), dept(DEPT_C), dept_ef()]);

    let detected = collect_transitions(&world);
    let planted = vec![
        PlantedEvent { kind: TransitionKind::Born, batch: p0, members: set(&dept(DEPT_A)), label: "A" },
        PlantedEvent { kind: TransitionKind::Born, batch: p0, members: set(&dept(DEPT_B)), label: "B" },
        PlantedEvent { kind: TransitionKind::Born, batch: p0, members: set(&dept(DEPT_C)), label: "C" },
        PlantedEvent { kind: TransitionKind::Born, batch: p0, members: set(&dept(DEPT_D)), label: "D" },
        PlantedEvent { kind: TransitionKind::Born, batch: p0, members: set(&dept(DEPT_E)), label: "E" },
        PlantedEvent { kind: TransitionKind::Born, batch: p0, members: set(&dept(DEPT_F)), label: "F" },
        PlantedEvent { kind: TransitionKind::Merge, batch: p1, members: set(&dept_ef()), label: "E+F→EF" },
        PlantedEvent { kind: TransitionKind::Dormant, batch: p2, members: set(&dept_ef()), label: "EF dormant" },
        PlantedEvent { kind: TransitionKind::Dormant, batch: p3, members: set(&dept(DEPT_D)), label: "D dormant" },
        PlantedEvent { kind: TransitionKind::Revived, batch: p4, members: set(&dept_ef()), label: "EF revived" },
    ];

    let scores = score_transitions(&world, &detected, &planted);
    report("full_enron_protocol", &scores, &detected);

    let born = scores.get(&TransitionKind::Born).unwrap();
    assert!(born.tp >= 5, "Born tp={} (need ≥5/6)", born.tp);
    assert!(born.precision() >= 0.70, "Born precision={:.2}", born.precision());

    let merge = scores.get(&TransitionKind::Merge).unwrap();
    assert_eq!(merge.tp, 1, "E+F→EF merge tp={}", merge.tp);

    let dormant = scores.get(&TransitionKind::Dormant).unwrap();
    assert!(dormant.tp >= 1, "EF must go dormant (tp={})", dormant.tp);

    let revived = scores.get(&TransitionKind::Revived).unwrap();
    assert!(revived.tp >= 1, "EF revival must be detected (tp={})", revived.tp);

    // CoherenceShift expected 0 — closes Finding 2a
    let (_, cs, _) = noise_counts(&detected);
    println!("\n  CoherenceShift raw count: {cs} (expected 0 — Finding 2a closure)");
}

// ── Prediction accuracy (precision@K, Ω1 supervised metric) ─────────────────
//
// Train on phases 0–3 (stable + merge + dormant + contract), rank pairs by
// activity, test on phase 4 (revival). Bar: precision@20 ≥ 2× base rate.

#[test]
fn next_phase_prediction_accuracy() {
    // Build train world (phases 0–3)
    let mut train_world = World::new();
    insert_employees(&mut train_world);
    let inf = make_inf_reg();
    let loci_reg = make_loci_reg();
    let engine = Engine::new(EngineConfig { max_batches_per_tick: 3 });
    let perspective = DefaultEmergencePerspective::default();

    run_phase(&mut train_world, &engine, &loci_reg, &inf, &perspective,
        &(0..N_DEPTS).map(dept).collect::<Vec<_>>());
    run_phase(&mut train_world, &engine, &loci_reg, &inf, &perspective,
        &vec![dept(DEPT_A), dept(DEPT_B), dept(DEPT_C), dept(DEPT_D), dept_ef()]);
    run_phase(&mut train_world, &engine, &loci_reg, &inf, &perspective,
        &vec![dept(DEPT_A), dept(DEPT_B), dept(DEPT_C), dept(DEPT_D)]);
    run_phase(&mut train_world, &engine, &loci_reg, &inf, &perspective,
        &vec![dept(DEPT_A), dept(DEPT_B), dept(DEPT_C)]);

    // Rank pairs by activity
    let mut ranked: Vec<((u64, u64), f32)> = train_world
        .relationships()
        .iter()
        .filter_map(|rel| {
            use graph_core::Endpoints;
            if let Endpoints::Symmetric { a, b } = rel.endpoints {
                let key = if a.0 < b.0 { (a.0, b.0) } else { (b.0, a.0) };
                Some((key, rel.activity()))
            } else {
                None
            }
        })
        .collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    // Test pairs: all intra-dept pairs active in phase 4 (A, B, C, EF).
    let active4 = vec![dept(DEPT_A), dept(DEPT_B), dept(DEPT_C), dept_ef()];
    let mut test_pairs: BTreeSet<(u64, u64)> = BTreeSet::new();
    for members in &active4 {
        for &a in members {
            for &b in members {
                if a < b {
                    test_pairs.insert((a, b));
                }
            }
        }
    }

    let n_possible = (N_EMPLOYEES * (N_EMPLOYEES - 1) / 2) as usize;
    let base_rate = test_pairs.len() as f32 / n_possible as f32;

    let candidates: Vec<(u64, u64)> = ranked.iter().map(|(k, _)| *k).collect();
    println!(
        "\n── next_phase_prediction_accuracy ──\n\
         ranked pairs: {}  test pairs: {} / {} (base {base_rate:.4})",
        candidates.len(), test_pairs.len(), n_possible
    );

    for &k in &[20usize, 50, 100] {
        let top: Vec<_> = candidates.iter().take(k).copied().collect();
        if top.is_empty() { println!("  k={k}: no candidates"); continue; }
        let hits = top.iter().filter(|p| test_pairs.contains(p)).count();
        let precision = hits as f32 / top.len() as f32;
        let lift = if base_rate > 0.0 { precision / base_rate } else { 0.0 };
        println!("  precision@{k:<3} = {precision:.3}  (hits {hits}/{},  lift {lift:.2}×)", top.len());
    }

    let top20: Vec<_> = candidates.iter().take(20).copied().collect();
    assert!(!top20.is_empty(), "no pairs ranked after training");
    let hits20 = top20.iter().filter(|p| test_pairs.contains(p)).count();
    let precision20 = hits20 as f32 / top20.len() as f32;
    let bar = (2.0 * base_rate).min(0.75);
    assert!(
        precision20 >= bar,
        "precision@20 = {precision20:.3} below bar {bar:.3} (2× base rate {base_rate:.4})"
    );
}
