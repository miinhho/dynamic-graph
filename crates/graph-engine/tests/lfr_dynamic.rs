//! Oracle test: Greene-style dynamic community benchmark.
//!
//! Canonical dynamic community-detection protocol (Greene, Doyle,
//! Cunningham, 2010, *Tracking the Evolution of Communities in Dynamic
//! Social Networks*). A fixed set of nodes is partitioned into planted
//! communities whose membership evolves through a schedule of **Birth**,
//! **Split**, **Merge**, and **Dormant** events; the test measures whether
//! the engine's `LayerTransition` stream recovers the planted transitions.
//!
//! Not a full LFR generator — real LFR (Lancichinetti-Fortunato-Radicchi)
//! 2009 planted-partition graphs add power-law degree / mixing params.
//! Those don't add signal for the question this test is built to answer:
//! *which of the 7 `LayerTransition` variants actually fire on a realistic
//! dynamic workload?* A planted-partition SBM with a Greene schedule is
//! sufficient.
//!
//! ## Scoring protocol (declared up-front so results are interpretable)
//!
//! A planted transition is considered **detected** iff the engine deposits
//! a `LayerTransition` for which all three conditions hold:
//!
//! 1. **Type match**   — planted.kind == detected.kind
//! 2. **Time window**  — |planted.batch − detected.batch| ≤ `MATCH_WINDOW`
//! 3. **Identity match** — Jaccard(planted.members, detected.members) ≥
//!    `MEMBER_JACCARD_MIN` (for Born/Dormant the entity's then-current
//!    members; for Split the offspring union; for Merge the absorbed
//!    entities' union)
//!
//! `precision = TP / detected(of kind)`; `recall = TP / planted(of kind)`.
//!
//! `MembershipDelta` and `CoherenceShift` are **excluded from scoring**
//! because they're drift noise, not planted events — but their raw counts
//! are reported to feed the complexity-audit "which transitions actually
//! fire" table (see `docs/complexity-audit.md`).
//!
//! ## Layout
//!
//! 60 nodes, 4 planted communities of 15:
//!   A = 0..15     B = 15..30    C = 30..45    D = 45..60
//!
//! The **isolated** tests (`planted_*_detected`) run one scenario each to
//! unambiguously verify a single transition type fires. The **composite**
//! test (`composite_greene_protocol_tuned`) runs the full 4-phase Greene
//! schedule and reports aggregate precision/recall.

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

const N_NODES: u64 = 60;

/// Planted communities.
const A: &[u64] = &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14];
const B: &[u64] = &[15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29];
const C: &[u64] = &[30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44];
const D: &[u64] = &[45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59];

/// Split children of A (8/7 so both offspring Jaccard-match A above 0.4).
const A1: &[u64] = &[0, 1, 2, 3, 4, 5, 6, 7];
const A2: &[u64] = &[8, 9, 10, 11, 12, 13, 14];

const CD: &[u64] = &[
    30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53,
    54, 55, 56, 57, 58, 59,
];

/// Decay half-life of ~1 batch — ghost edges from a previous phase drop
/// below threshold after ~3–4 batches of silence.
const DECAY: f32 = 0.5;
const BATCHES_PER_PHASE: usize = 4;

/// At activity threshold 0.3, intra-community edges (steady state ~2.0)
/// clear; ghost cross-community edges (≤ 2·0.5⁴ = 0.125) fall below.
const ACTIVITY_THRESHOLD: f32 = 0.3;

/// Oracle: planted event matches a detected transition within this many
/// batches. Wide enough that "detected at the end of the phase" counts as
/// on-time.
const MATCH_WINDOW: u64 = BATCHES_PER_PHASE as u64 + 2;

/// Oracle: member-set Jaccard threshold for identity matching. Stricter
/// than the engine's own reconciliation so the test measures the engine's
/// choices, not the oracle's tolerance.
const MEMBER_JACCARD_MIN: f32 = 0.5;

const CO_ACT: InfluenceKindId = InfluenceKindId(200);
const NODE_KIND: LocusKindId = LocusKindId(1);

// ── Co-activation program (Davis-pattern, depth-1 broadcast) ─────────────────

struct CoActivationProgram;

impl LocusProgram for CoActivationProgram {
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
                let PropertyValue::Int(id) = val else {
                    continue;
                };
                let other = *id as u64;
                if other == locus.id.0 {
                    continue;
                }
                out.push(ProposedChange::new(
                    ChangeSubject::Locus(LocusId(other)),
                    CO_ACT,
                    StateVector::from_slice(&[1.0]),
                ));
            }
        }
        out
    }
}

// ── World setup ──────────────────────────────────────────────────────────────

fn insert_nodes(world: &mut World) {
    for i in 0..N_NODES {
        world.insert_locus(Locus::new(LocusId(i), NODE_KIND, StateVector::zeros(1)));
    }
}

fn make_inf_reg() -> InfluenceKindRegistry {
    let cfg = InfluenceKindConfig::new("co_activation")
        .with_decay(DECAY)
        .with_symmetric(true);
    let mut reg = InfluenceKindRegistry::new();
    reg.insert(CO_ACT, cfg);
    reg
}

fn make_loci_reg() -> LocusKindRegistry {
    let mut reg = LocusKindRegistry::new();
    reg.insert(NODE_KIND, Box::new(CoActivationProgram));
    reg
}

/// One co-activation event = one `ProposedChange` per member carrying the
/// other members in its `co_members` metadata. Exactly mirrors the Davis
/// pattern.
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
                CO_ACT,
                StateVector::from_slice(&[1.0]),
            )
            .with_metadata(meta)
        })
        .collect()
}

// ── Phase runner ─────────────────────────────────────────────────────────────

/// A **phase** is "which planted communities are active right now". Each
/// batch within the phase fires one co-activation event per active
/// community.
///
/// Returns `(world, final_batch)` so callers can attribute detected
/// transitions to the right phase.
fn run_phases(
    phases: &[&[&[u64]]],
    perspective: &DefaultEmergencePerspective,
) -> (World, Vec<BatchId>) {
    let mut world = World::new();
    insert_nodes(&mut world);
    let inf = make_inf_reg();
    let loci_reg = make_loci_reg();
    // max_batches_per_tick = 3: stimulus → per-locus program → auto-emerge.
    let engine = Engine::new(EngineConfig {
        max_batches_per_tick: 3,
    });

    let mut phase_end_batches = Vec::new();
    for active in phases {
        for _ in 0..BATCHES_PER_PHASE {
            let mut stim = Vec::new();
            for community in *active {
                stim.extend(community_event(community));
            }
            engine.tick(&mut world, &loci_reg, &inf, stim);
        }
        engine.recognize_entities(&mut world, &inf, perspective);
        phase_end_batches.push(world.current_batch());
    }
    (world, phase_end_batches)
}

// ── Transition stream extraction ─────────────────────────────────────────────

/// A single layer deposit flattened into a comparable record.
#[derive(Debug, Clone)]
struct DetectedTransition {
    entity: EntityId,
    batch: BatchId,
    kind: TransitionKind,
    /// Members carried by the transition — Born/Dormant: the entity's
    /// snapshot at that layer; Split: union of offspring members; Merge:
    /// this entity is the absorbed side, so its snapshot is the members
    /// that got folded in.
    members: BTreeSet<u64>,
    /// Offspring (Split) or absorbed (Merge) ids, when relevant.
    related: Vec<EntityId>,
}

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

/// One `Merge` *proposal* deposits two `Merged` layers (survivor + absorbed);
/// one `Split` *proposal* deposits two `Born` layers (one per offspring) plus
/// one `Split` layer. For scoring to reflect proposal-level precision (what
/// the engine "decides"), derived layers must be filtered out:
///
/// - **Absorbed-side `Merged`**: the owning entity was marked Dormant at the
///   same batch by the merge (and has no later Revived). Kept as scoring noise
///   would triple-count every merge.
/// - **Offspring `Born`**: a `Split` at the same batch listed this entity as
///   offspring. Not an independent birth — part of the parent's split.
fn collect_transitions(world: &World) -> Vec<DetectedTransition> {
    let mut all = Vec::new();
    // (offspring_entity, batch) produced by a Split — exclude from Born count.
    let mut split_offspring: std::collections::HashSet<(EntityId, BatchId)> =
        std::collections::HashSet::new();
    // (absorbed_entity, batch) produced by a Merge — exclude one of the two
    // Merged layers. We identify absorbed-side by member-count: the survivor's
    // snapshot carries the merged union, so it has strictly more members than
    // the individual absorbed snapshots.
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
                // Survivor stays Active; absorbed entity is marked Dormant by
                // the engine at the merge batch. A Merged layer on a Dormant
                // owner is therefore the absorbed-side deposit.
                //
                // Edge case: an entity could be the survivor of a merge and
                // later go dormant via `BecameDormant`. That only misfilters
                // if the dormancy happens in the same test run after the
                // merge — our isolated tests don't do this, and the composite
                // test's final dormancy concerns B, which is not a merge
                // survivor. If SocioPatterns/Enron exercise the chained case,
                // switch to per-batch status via the layer stack.
                merge_absorbed.insert((e.id, layer.batch));
            }
        }
    }

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
                LayerTransition::MembershipDelta { .. } => {
                    (TransitionKind::MembershipDelta, vec![])
                }
                LayerTransition::CoherenceShift { .. } => (TransitionKind::CoherenceShift, vec![]),
                LayerTransition::Revived => (TransitionKind::Revived, vec![]),
            };
            all.push(DetectedTransition {
                entity: e.id,
                batch: layer.batch,
                kind,
                members,
                related,
            });
        }
    }
    all.sort_by_key(|t| (t.batch.0, t.entity.0));
    all
}

/// For Split, the "identity" of the event is the union of offspring
/// members, computed by looking up the offspring entities' then-current
/// snapshots. Similarly for Merge: union of absorbed entities' members.
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
    if union == 0 {
        1.0
    } else {
        inter as f32 / union as f32
    }
}

// ── Oracle ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct PlantedEvent {
    kind: TransitionKind,
    batch: BatchId,
    members: BTreeSet<u64>,
    #[allow(dead_code)] // carried for Debug output when a test fails
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
        if self.detected == 0 {
            0.0
        } else {
            self.tp as f32 / self.detected as f32
        }
    }
    fn recall(&self) -> f32 {
        if self.planted == 0 {
            0.0
        } else {
            self.tp as f32 / self.planted as f32
        }
    }
}

/// Greedy matching: each planted event is matched to at most one detected
/// transition of the same kind (whichever is closest in time and passes
/// Jaccard). Double-counts are avoided by consuming matched detections.
fn score_transitions(
    world: &World,
    detected: &[DetectedTransition],
    planted: &[PlantedEvent],
) -> std::collections::HashMap<TransitionKind, Score> {
    use std::collections::HashMap;

    let mut by_kind: HashMap<TransitionKind, Score> = HashMap::new();
    for k in [
        TransitionKind::Born,
        TransitionKind::Split,
        TransitionKind::Merge,
        TransitionKind::Dormant,
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
            if consumed[i] || d.kind != p.kind {
                continue;
            }
            let dt = (d.batch.0 as i64 - p.batch.0 as i64).abs();
            if dt as u64 > MATCH_WINDOW {
                continue;
            }
            let d_members = effective_members(world, d);
            let score = jaccard(&p.members, &d_members);
            if score < MEMBER_JACCARD_MIN {
                continue;
            }
            let better = match best {
                None => true,
                Some((_, b_dt, b_score)) => dt < b_dt || (dt == b_dt && score > b_score),
            };
            if better {
                best = Some((i, dt, score));
            }
        }
        if let Some((i, _, _)) = best {
            consumed[i] = true;
            if let Some(s) = by_kind.get_mut(&p.kind) {
                s.tp += 1;
            }
        }
    }
    by_kind
}

/// Raw counts including non-scored kinds — feeds the complexity-audit
/// "which transitions fire" table.
fn noise_counts(detected: &[DetectedTransition]) -> (usize, usize, usize) {
    let mut membership_delta = 0;
    let mut coherence_shift = 0;
    let mut revived = 0;
    for d in detected {
        match d.kind {
            TransitionKind::MembershipDelta => membership_delta += 1,
            TransitionKind::CoherenceShift => coherence_shift += 1,
            TransitionKind::Revived => revived += 1,
            _ => {}
        }
    }
    (membership_delta, coherence_shift, revived)
}

fn report(
    label: &str,
    scores: &std::collections::HashMap<TransitionKind, Score>,
    detected: &[DetectedTransition],
) {
    let (md, cs, rv) = noise_counts(detected);
    println!("\n── {label} ────────────────────────────────────");
    println!(
        "{:<18} {:>8} {:>8} {:>8} {:>10} {:>10}",
        "kind", "planted", "detected", "tp", "precision", "recall"
    );
    for k in [
        TransitionKind::Born,
        TransitionKind::Split,
        TransitionKind::Merge,
        TransitionKind::Dormant,
    ] {
        let s = scores.get(&k).unwrap();
        println!(
            "{:<18} {:>8} {:>8} {:>8} {:>10.2} {:>10.2}",
            format!("{:?}", k),
            s.planted,
            s.detected,
            s.tp,
            s.precision(),
            s.recall()
        );
    }
    println!("{:<18} {:>8} {:>8}", "MembershipDelta", "n/a", md);
    println!("{:<18} {:>8} {:>8}", "CoherenceShift", "n/a", cs);
    println!("{:<18} {:>8} {:>8}", "Revived", "n/a", rv);
}

fn set(xs: &[u64]) -> BTreeSet<u64> {
    xs.iter().copied().collect()
}

// ── Sanity: data integrity ───────────────────────────────────────────────────

#[test]
fn planted_layout_is_disjoint_and_complete() {
    let mut all = BTreeSet::new();
    for group in [A, B, C, D] {
        for &n in group {
            assert!(all.insert(n), "duplicate node {n} in planted layout");
        }
    }
    assert_eq!(all.len(), N_NODES as usize);
    // A1 ∪ A2 == A
    let mut a_children = BTreeSet::new();
    for &n in A1.iter().chain(A2.iter()) {
        assert!(a_children.insert(n));
    }
    assert_eq!(a_children, set(A));
    // CD == C ∪ D
    let mut cd_set = BTreeSet::new();
    for &n in C.iter().chain(D.iter()) {
        cd_set.insert(n);
    }
    assert_eq!(cd_set, set(CD));
}

// ── Isolated: Born ───────────────────────────────────────────────────────────

/// Seed four communities in parallel; expect the engine to deposit a
/// `Born` layer for each. Baseline of detection — if this fails, no other
/// transition test is meaningful.
#[test]
fn planted_born_detected() {
    let perspective = DefaultEmergencePerspective {
        min_activity_threshold: Some(ACTIVITY_THRESHOLD),
    };
    let (world, phase_ends) = run_phases(&[&[A, B, C, D]], &perspective);

    let detected = collect_transitions(&world);
    let planted = vec![
        PlantedEvent {
            kind: TransitionKind::Born,
            batch: phase_ends[0],
            members: set(A),
            label: "A",
        },
        PlantedEvent {
            kind: TransitionKind::Born,
            batch: phase_ends[0],
            members: set(B),
            label: "B",
        },
        PlantedEvent {
            kind: TransitionKind::Born,
            batch: phase_ends[0],
            members: set(C),
            label: "C",
        },
        PlantedEvent {
            kind: TransitionKind::Born,
            batch: phase_ends[0],
            members: set(D),
            label: "D",
        },
    ];
    let scores = score_transitions(&world, &detected, &planted);
    report("planted_born_detected", &scores, &detected);

    let born = scores.get(&TransitionKind::Born).unwrap();
    assert_eq!(born.tp, 4, "all 4 planted Born events must be detected");
    assert!(born.precision() >= 0.75, "too many spurious Born events");
}

// ── Isolated: Split ──────────────────────────────────────────────────────────

/// Phase 0: A active. Phase 1: only A1 and A2 active (no cross-A1-A2
/// stimulation). The engine should deposit a `Split` layer on entity A
/// with 2 offspring.
#[test]
fn planted_split_detected() {
    let perspective = DefaultEmergencePerspective {
        min_activity_threshold: Some(ACTIVITY_THRESHOLD),
    };
    let (world, phase_ends) = run_phases(&[&[A], &[A1, A2]], &perspective);

    let detected = collect_transitions(&world);
    let planted = vec![
        PlantedEvent {
            kind: TransitionKind::Born,
            batch: phase_ends[0],
            members: set(A),
            label: "A",
        },
        PlantedEvent {
            kind: TransitionKind::Split,
            batch: phase_ends[1],
            // Union of the split offspring.
            members: set(A),
            label: "A → A1+A2",
        },
    ];
    let scores = score_transitions(&world, &detected, &planted);
    report("planted_split_detected", &scores, &detected);

    let split = scores.get(&TransitionKind::Split).unwrap();
    assert_eq!(split.tp, 1, "the A → A1+A2 split must be detected");
}

// ── Isolated: Merge ──────────────────────────────────────────────────────────

/// Phase 0: C and D separately active. Phase 1: CD stimulated as a single
/// community. The engine should deposit a `Merged` layer.
#[test]
fn planted_merge_detected() {
    let perspective = DefaultEmergencePerspective {
        min_activity_threshold: Some(ACTIVITY_THRESHOLD),
    };
    let (world, phase_ends) = run_phases(&[&[C, D], &[CD]], &perspective);

    let detected = collect_transitions(&world);
    let planted = vec![
        PlantedEvent {
            kind: TransitionKind::Born,
            batch: phase_ends[0],
            members: set(C),
            label: "C",
        },
        PlantedEvent {
            kind: TransitionKind::Born,
            batch: phase_ends[0],
            members: set(D),
            label: "D",
        },
        PlantedEvent {
            kind: TransitionKind::Merge,
            batch: phase_ends[1],
            members: set(CD),
            label: "C+D → CD",
        },
    ];
    let scores = score_transitions(&world, &detected, &planted);
    report("planted_merge_detected", &scores, &detected);

    let merge = scores.get(&TransitionKind::Merge).unwrap();
    assert_eq!(merge.tp, 1, "the C+D merge must be detected");
}

// ── Isolated: Dormant ────────────────────────────────────────────────────────

/// Phase 0: B active (also A seeded so the entity store isn't empty —
/// avoids degenerate cases). Phase 1: A continues, B silent. B's intra
/// activity decays below threshold; the engine should mark B dormant.
#[test]
fn planted_dormant_detected() {
    let perspective = DefaultEmergencePerspective {
        min_activity_threshold: Some(ACTIVITY_THRESHOLD),
    };
    let (world, phase_ends) = run_phases(&[&[A, B], &[A]], &perspective);

    let detected = collect_transitions(&world);
    let planted = vec![
        PlantedEvent {
            kind: TransitionKind::Born,
            batch: phase_ends[0],
            members: set(A),
            label: "A",
        },
        PlantedEvent {
            kind: TransitionKind::Born,
            batch: phase_ends[0],
            members: set(B),
            label: "B",
        },
        PlantedEvent {
            kind: TransitionKind::Dormant,
            batch: phase_ends[1],
            members: set(B),
            label: "B dormant",
        },
    ];
    let scores = score_transitions(&world, &detected, &planted);
    report("planted_dormant_detected", &scores, &detected);

    let dormant = scores.get(&TransitionKind::Dormant).unwrap();
    assert_eq!(
        dormant.tp, 1,
        "B must go dormant after its phase of silence"
    );

    // Sanity: the entity is indeed dormant in the store.
    let b_entity = world
        .entities()
        .iter()
        .find(|e| {
            e.current.members.iter().all(|l| B.contains(&l.0)) && e.current.members.len() == B.len()
        })
        .expect("B entity not found");
    assert_eq!(b_entity.status, EntityStatus::Dormant);
}

// ── Composite: full Greene protocol ──────────────────────────────────────────

/// Full 4-phase Greene protocol: Born (×4) → Split → Merge → Dormant.
/// With tuned `overlap_threshold` and the Split-source-dormancy fix
/// (2026-04-18, `apply_emergence` Split branch), all four planted
/// transitions recover at precision 1.0 / recall 1.0.
#[test]
fn composite_greene_protocol_tuned() {
    let perspective = DefaultEmergencePerspective {
        min_activity_threshold: Some(ACTIVITY_THRESHOLD),
    };
    let (world, phase_ends) = run_phases(
        &[
            &[A, B, C, D],      // phase 0: seed
            &[A1, A2, B, C, D], // phase 1: split A
            &[A1, A2, B, CD],   // phase 2: merge CD
            &[A1, A2, CD],      // phase 3: dormant B
        ],
        &perspective,
    );

    let detected = collect_transitions(&world);
    let planted = vec![
        PlantedEvent {
            kind: TransitionKind::Born,
            batch: phase_ends[0],
            members: set(A),
            label: "A born",
        },
        PlantedEvent {
            kind: TransitionKind::Born,
            batch: phase_ends[0],
            members: set(B),
            label: "B born",
        },
        PlantedEvent {
            kind: TransitionKind::Born,
            batch: phase_ends[0],
            members: set(C),
            label: "C born",
        },
        PlantedEvent {
            kind: TransitionKind::Born,
            batch: phase_ends[0],
            members: set(D),
            label: "D born",
        },
        PlantedEvent {
            kind: TransitionKind::Split,
            batch: phase_ends[1],
            members: set(A),
            label: "A → A1+A2",
        },
        PlantedEvent {
            kind: TransitionKind::Merge,
            batch: phase_ends[2],
            members: set(CD),
            label: "C+D → CD",
        },
        PlantedEvent {
            kind: TransitionKind::Dormant,
            batch: phase_ends[3],
            members: set(B),
            label: "B dormant",
        },
    ];
    let scores = score_transitions(&world, &detected, &planted);
    report("composite_greene_protocol_tuned", &scores, &detected);

    // Strict bar: the Split-source-dormancy fix means the only spurious
    // path (source re-matching its own children as a Merge) no longer
    // fires. Every planted transition should land with precision and
    // recall both 1.0.
    for k in [
        TransitionKind::Born,
        TransitionKind::Split,
        TransitionKind::Merge,
        TransitionKind::Dormant,
    ] {
        let s = scores.get(&k).unwrap();
        assert_eq!(
            s.recall(),
            1.0,
            "{k:?}: recall must be 1.0; got {} (tp={}, planted={})",
            s.recall(),
            s.tp,
            s.planted
        );
        assert_eq!(
            s.precision(),
            1.0,
            "{k:?}: precision must be 1.0 post-fix; got {} (tp={}, detected={})",
            s.precision(),
            s.tp,
            s.detected
        );
    }
}

// `composite_greene_protocol_default_threshold_recovers_most` removed —
// it asserted Finding 2b's broken Split behaviour (Jaccard 0.5 cutoff
// rejecting the 8/7 split). With the locus-flow rewrite that overlap
// knob no longer exists; the composite_tuned test above is now the
// single source of truth for the full protocol.
