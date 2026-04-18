//! Oracle test: SocioPatterns primary-school temporal contact network.
//!
//! Stehlé et al. (2011), *High-Resolution Measurements of Face-to-Face
//! Contact Patterns in a Primary School*, captured RFID proximity data at
//! 20 s resolution among 232 children across 10 classes over one school
//! day. Classes dominate intra-class contact probability; lunch breaks mix
//! children across classes.
//!
//! This test does NOT ingest the real dataset (too large, requires network
//! access). It synthesises a SocioPatterns-style stream with the same
//! qualitative structure at a scale appropriate for a unit test:
//!
//! - 40 students, 5 classes of 8 (vs the paper's 232 / 10)
//! - 60 time blocks, each emitting k co-attendance events
//! - `p_in` (intra-class) >> `p_out` (cross-class); one lunch block every
//!   10 raises `p_out` to imitate cross-class mixing
//! - Deterministic LCG seeded from a constant — no `rand`, no `std::time`
//!
//! ## Three tests
//!
//! 1. `planted_classes_are_recovered` — full stream → `recognize_entities`
//!    → assert ≥ 4/5 planted classes recovered at Jaccard ≥ 0.6.
//! 2. `time_block_stability` — run the stream in segments, record the
//!    active-entity count after each segment; assert the last few
//!    checkpoints stabilise (variance under a small bound).
//! 3. `next_block_prediction_accuracy` — train on 45 blocks, predict the
//!    top-K most-likely co-attending pairs for the next 15 blocks from
//!    relationship activity, compute precision@K. This is the candidate
//!    **supervised metric** for Phase-9 plasticity auto-tuning (see
//!    `docs/complexity-audit.md` Phase 9 scouting / reopen condition (a)).
//!
//! ## Auto-threshold
//!
//! `DefaultEmergencePerspective::min_activity_threshold` is `Option<f32>`
//! in main (Phase 2, gap detector scans lower 75% after the SocioPatterns
//! finding). Tests leave it `None` so the engine drives the heuristic
//! end-to-end. For diagnostics, each test prints the effective threshold
//! computed by a local helper that mirrors the engine's algorithm.

use graph_core::{
    ChangeSubject, Endpoints, InfluenceKindId, Locus, LocusContext, LocusId, LocusKindId,
    LocusProgram, Properties, PropertyValue, ProposedChange, StateVector,
};
use graph_engine::{
    DefaultEmergencePerspective, Engine, EngineConfig, InfluenceKindConfig, InfluenceKindRegistry,
    LocusKindRegistry,
};
use graph_world::World;
use std::collections::BTreeSet;

// ── Constants ────────────────────────────────────────────────────────────────

const N_STUDENTS: u64 = 40;
const CLASS_SIZE: u64 = 8;
const N_CLASSES: u64 = 5; // 5 × 8 = 40

const N_BLOCKS: usize = 60;
/// Number of co-attendance events per block.
const EVENTS_PER_BLOCK: usize = 10;
/// Students present at each event.
const GROUP_SIZE_MIN: usize = 4;
const GROUP_SIZE_MAX: usize = 6;

/// Intra-class pair selection probability (normalised into pair sampling).
const P_IN: f32 = 0.75;
/// Cross-class pair selection probability during non-lunch blocks.
const P_OUT: f32 = 0.05;
/// During lunch blocks, cross-class probability rises (mixing).
const P_OUT_LUNCH: f32 = 0.25;
/// Every 10th block is a "lunch" block with increased cross-class mixing.
const LUNCH_EVERY: usize = 10;

/// Activity decays ~0.92 per batch.
const DECAY: f32 = 0.92;

/// Deterministic seed for the LCG.
const SEED: u64 = 0x50c10_5ca77e5d;

const CO_ATTEND: InfluenceKindId = InfluenceKindId(300);
const STUDENT_KIND: LocusKindId = LocusKindId(1);

// ── Deterministic pseudo-random (LCG) ────────────────────────────────────────

#[derive(Debug, Clone)]
struct Lcg(u64);

impl Lcg {
    fn new(seed: u64) -> Self {
        Self(seed)
    }
    fn next_u64(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0
    }
    fn next_f32(&mut self) -> f32 {
        ((self.next_u64() >> 40) as f32) / ((1u64 << 24) as f32)
    }
    fn next_range(&mut self, min: usize, max_inclusive: usize) -> usize {
        let span = (max_inclusive - min + 1) as u64;
        min + (self.next_u64() % span) as usize
    }
    fn choose_in(&mut self, xs: &[u64]) -> u64 {
        xs[(self.next_u64() as usize) % xs.len()]
    }
}

// ── Synthetic dataset generator ──────────────────────────────────────────────

fn class_of(student: u64) -> u64 {
    student / CLASS_SIZE
}

fn class_members(class: u64) -> Vec<u64> {
    (class * CLASS_SIZE..(class + 1) * CLASS_SIZE).collect()
}

fn all_classes() -> Vec<Vec<u64>> {
    (0..N_CLASSES).map(class_members).collect()
}

fn sample_event(rng: &mut Lcg, block_idx: usize) -> Vec<u64> {
    let lunch = block_idx > 0 && block_idx % LUNCH_EVERY == 0;
    let p_out = if lunch { P_OUT_LUNCH } else { P_OUT };
    let classes = all_classes();

    let focal = rng.next_u64() % N_STUDENTS;
    let focal_class = class_of(focal);
    let mut attendees = BTreeSet::new();
    attendees.insert(focal);

    let group_size = rng.next_range(GROUP_SIZE_MIN, GROUP_SIZE_MAX);
    let mut guard = 0;
    while attendees.len() < group_size && guard < group_size * 8 {
        guard += 1;
        let draw_intra = rng.next_f32() < (P_IN / (P_IN + p_out));
        let pool = if draw_intra {
            classes[focal_class as usize].as_slice()
        } else {
            let other_class = {
                let mut c = rng.next_u64() % N_CLASSES;
                if c == focal_class {
                    c = (c + 1) % N_CLASSES;
                }
                c
            };
            classes[other_class as usize].as_slice()
        };
        let pick = rng.choose_in(pool);
        attendees.insert(pick);
    }

    attendees.into_iter().collect()
}

fn block_events(rng: &mut Lcg, block_idx: usize) -> Vec<Vec<u64>> {
    (0..EVENTS_PER_BLOCK)
        .map(|_| sample_event(rng, block_idx))
        .collect()
}

// ── Co-attendance program (depth-1 broadcast, identical pattern to Davis) ────

struct CoAttendanceProgram;

impl LocusProgram for CoAttendanceProgram {
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
            let Some(PropertyValue::List(ids)) = meta.get("co_attendees") else {
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
                    CO_ATTEND,
                    StateVector::from_slice(&[1.0]),
                ));
            }
        }
        out
    }
}

fn event_stimuli(attendees: &[u64]) -> Vec<ProposedChange> {
    attendees
        .iter()
        .map(|&w| {
            let others: Vec<PropertyValue> = attendees
                .iter()
                .filter(|&&x| x != w)
                .map(|&id| PropertyValue::Int(id as i64))
                .collect();
            let mut meta = Properties::new();
            meta.set("co_attendees", PropertyValue::List(others));
            ProposedChange::new(
                ChangeSubject::Locus(LocusId(w)),
                CO_ATTEND,
                StateVector::from_slice(&[1.0]),
            )
            .with_metadata(meta)
        })
        .collect()
}

// ── World / registry setup ───────────────────────────────────────────────────

fn insert_students(world: &mut World) {
    for i in 0..N_STUDENTS {
        world.insert_locus(Locus::new(LocusId(i), STUDENT_KIND, StateVector::zeros(1)));
    }
}

fn make_inf_reg() -> InfluenceKindRegistry {
    let cfg = InfluenceKindConfig::new("co_attendance")
        .with_decay(DECAY)
        .with_symmetric(true);
    let mut reg = InfluenceKindRegistry::new();
    reg.insert(CO_ATTEND, cfg);
    reg
}

fn make_loci_reg() -> LocusKindRegistry {
    let mut reg = LocusKindRegistry::new();
    reg.insert(STUDENT_KIND, Box::new(CoAttendanceProgram));
    reg
}

fn run_stream(
    blocks_to_run: usize,
    seed: u64,
) -> (World, InfluenceKindRegistry, Vec<Vec<Vec<u64>>>) {
    let mut world = World::new();
    insert_students(&mut world);
    let inf = make_inf_reg();
    let loci_reg = make_loci_reg();

    let engine = Engine::new(EngineConfig {
        max_batches_per_tick: 3,
    });
    let mut rng = Lcg::new(seed);

    let mut event_log = Vec::with_capacity(blocks_to_run);
    for b in 0..blocks_to_run {
        let events = block_events(&mut rng, b);
        let mut stimuli = Vec::new();
        for ev in &events {
            if ev.len() >= 2 {
                stimuli.extend(event_stimuli(ev));
            }
        }
        if !stimuli.is_empty() {
            engine.tick(&mut world, &loci_reg, &inf, stimuli);
        }
        event_log.push(events);
    }
    (world, inf, event_log)
}

// ── Diagnostics: mirror the engine's auto-threshold for printing ─────────────

/// Returns the threshold the engine's `DefaultEmergencePerspective` would
/// compute for this world, replicating the lower-75% largest-relative-gap
/// heuristic in `emergence/default.rs::auto_activity_threshold`. Used only
/// for diagnostic output; tests themselves rely on the engine's own
/// computation by passing `min_activity_threshold: None`.
fn diagnostic_threshold(world: &World) -> f32 {
    let mut activities: Vec<f32> = world
        .relationships()
        .iter()
        .map(|r| r.activity().abs())
        .filter(|&a| a > 0.0)
        .collect();
    if activities.len() < 4 {
        return 0.0;
    }
    activities.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let search_end = activities.len() * 3 / 4 + 1;
    let mut best_gap_ratio = 1.0f32;
    let mut best_threshold = 0.0f32;
    for i in 0..search_end.saturating_sub(1) {
        let a = activities[i];
        let b = activities[i + 1];
        if a < 1e-6 {
            continue;
        }
        let ratio = b / a;
        if ratio > best_gap_ratio {
            best_gap_ratio = ratio;
            best_threshold = a * 1.0001;
        }
    }
    if best_gap_ratio >= 2.0 {
        best_threshold
    } else {
        0.0
    }
}

fn print_activity_distribution(world: &World, label: &str) {
    let mut acts: Vec<f32> = world
        .relationships()
        .iter()
        .map(|r| r.activity())
        .filter(|a| *a > 0.0)
        .collect();
    if acts.is_empty() {
        println!("  [activity dist/{label}] n=0");
        return;
    }
    acts.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = acts.len();
    println!(
        "  [activity dist/{label}] n={n} min={:.2} p25={:.2} p50={:.2} p75={:.2} p90={:.2} max={:.2}",
        acts[0],
        acts[n / 4],
        acts[n / 2],
        acts[n * 3 / 4],
        acts[n * 9 / 10],
        acts[n - 1],
    );
}

// ── Oracle helpers ───────────────────────────────────────────────────────────

fn jaccard(a: &BTreeSet<u64>, b: &BTreeSet<u64>) -> f32 {
    let inter = a.intersection(b).count();
    let union = a.len() + b.len() - inter;
    if union == 0 {
        1.0
    } else {
        inter as f32 / union as f32
    }
}

struct Recovery {
    jaccard: f32,
    precision: f32,
    recall: f32,
}

fn score_recovery(world: &World, classes: &[Vec<u64>]) -> Vec<Recovery> {
    let entities: Vec<BTreeSet<u64>> = world
        .entities()
        .active()
        .map(|e| e.current.members.iter().map(|l| l.0).collect())
        .collect();
    let class_sets: Vec<BTreeSet<u64>> = classes
        .iter()
        .map(|c| c.iter().copied().collect())
        .collect();

    let mut pairs: Vec<(usize, usize, f32)> = Vec::new();
    for (ci, c) in class_sets.iter().enumerate() {
        for (ei, e) in entities.iter().enumerate() {
            pairs.push((ci, ei, jaccard(c, e)));
        }
    }
    pairs.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));

    let mut class_assigned = vec![false; class_sets.len()];
    let mut entity_assigned = vec![false; entities.len()];
    let mut out: Vec<Recovery> = (0..class_sets.len())
        .map(|_| Recovery {
            jaccard: 0.0,
            precision: 0.0,
            recall: 0.0,
        })
        .collect();

    for (ci, ei, j) in pairs {
        if class_assigned[ci] || entity_assigned[ei] {
            continue;
        }
        let c = &class_sets[ci];
        let e = &entities[ei];
        let inter = c.intersection(e).count();
        out[ci] = Recovery {
            jaccard: j,
            precision: if e.is_empty() {
                0.0
            } else {
                inter as f32 / e.len() as f32
            },
            recall: if c.is_empty() {
                0.0
            } else {
                inter as f32 / c.len() as f32
            },
        };
        class_assigned[ci] = true;
        entity_assigned[ei] = true;
    }
    out
}

// ── Test 1: planted classes are recovered ────────────────────────────────────

#[test]
fn planted_classes_are_recovered() {
    let (mut world, inf, _log) = run_stream(N_BLOCKS, SEED);

    print_activity_distribution(&world, "full-stream");
    let diag_threshold = diagnostic_threshold(&world);

    let engine = Engine::default();
    // Phase 2 auto: leave threshold None so the engine picks the cutoff
    // via its lower-75% gap detector.
    let perspective = DefaultEmergencePerspective::default();
    engine.recognize_entities(&mut world, &inf, &perspective);

    let classes = all_classes();
    let recovered = score_recovery(&world, &classes);

    println!(
        "\n── planted_classes_are_recovered (diag_threshold={:.3}, entities={}) ──",
        diag_threshold,
        world.entities().active().count()
    );
    let mut ok = 0;
    for (ci, r) in recovered.iter().enumerate() {
        println!(
            "  class {ci}: jaccard={:.2} precision={:.2} recall={:.2}",
            r.jaccard, r.precision, r.recall
        );
        if r.jaccard >= 0.6 && r.precision >= 0.7 {
            ok += 1;
        }
    }
    println!(
        "  {ok}/{} classes recovered at jaccard≥0.6, precision≥0.7",
        classes.len()
    );

    assert!(
        ok >= 4,
        "expected ≥4/5 planted classes recovered (got {ok}); \
         if the generator changed, update this bar and rerun"
    );
}

// ── Test 2: time-block stability ─────────────────────────────────────────────

#[test]
fn time_block_stability() {
    const CHECKPOINT_EVERY: usize = 10;

    let mut world = World::new();
    insert_students(&mut world);
    let inf = make_inf_reg();
    let loci_reg = make_loci_reg();
    let engine = Engine::new(EngineConfig {
        max_batches_per_tick: 3,
    });
    let mut rng = Lcg::new(SEED);

    let mut counts: Vec<usize> = Vec::new();
    for b in 0..N_BLOCKS {
        let mut stimuli = Vec::new();
        for ev in block_events(&mut rng, b) {
            if ev.len() >= 2 {
                stimuli.extend(event_stimuli(&ev));
            }
        }
        if !stimuli.is_empty() {
            engine.tick(&mut world, &loci_reg, &inf, stimuli);
        }
        if (b + 1) % CHECKPOINT_EVERY == 0 {
            let diag = diagnostic_threshold(&world);
            let perspective = DefaultEmergencePerspective::default();
            let mut snapshot = world.clone();
            engine.recognize_entities(&mut snapshot, &inf, &perspective);
            let n_entities = snapshot.entities().active().count();
            counts.push(n_entities);
            println!(
                "  block {:>2} : diag_threshold={:.3}, entities={n_entities}",
                b + 1,
                diag
            );
        }
    }

    assert!(counts.len() >= 4, "need at least 4 checkpoints");
    let tail = &counts[counts.len() - 4..];
    let mean: f32 = tail.iter().map(|&x| x as f32).sum::<f32>() / tail.len() as f32;
    let var: f32 = tail
        .iter()
        .map(|&x| {
            let d = x as f32 - mean;
            d * d
        })
        .sum::<f32>()
        / tail.len() as f32;

    println!("  tail counts={tail:?}  mean={mean:.2}  var={var:.3}");
    assert!(
        var <= 2.0,
        "partition should stabilise by end of stream; tail variance {var:.3} too high"
    );
}

// ── Test 3: next-block prediction accuracy (Phase 9 reopen probe) ────────────

#[test]
fn next_block_prediction_accuracy() {
    const TRAIN_BLOCKS: usize = 45;
    const TEST_BLOCKS: usize = 15;
    const TOP_KS: [usize; 3] = [20, 50, 100];

    let (_world, _inf, event_log) = run_stream(TRAIN_BLOCKS + TEST_BLOCKS, SEED);

    let (train_world, _train_inf, _train_log) = run_stream(TRAIN_BLOCKS, SEED);
    let train_threshold = diagnostic_threshold(&train_world);
    print_activity_distribution(&train_world, "train");

    let mut ranked: Vec<((u64, u64), f32)> = Vec::new();
    for rel in train_world.relationships().iter() {
        if let Endpoints::Symmetric { a, b } = rel.endpoints {
            let key = if a.0 < b.0 { (a.0, b.0) } else { (b.0, a.0) };
            ranked.push((key, rel.activity()));
        }
    }
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let mut test_pairs: BTreeSet<(u64, u64)> = BTreeSet::new();
    for block in &event_log[TRAIN_BLOCKS..] {
        for ev in block {
            for i in 0..ev.len() {
                for j in (i + 1)..ev.len() {
                    let (a, b) = (ev[i], ev[j]);
                    test_pairs.insert(if a < b { (a, b) } else { (b, a) });
                }
            }
        }
    }
    let n_possible = (N_STUDENTS * (N_STUDENTS - 1) / 2) as usize;
    let base_rate = test_pairs.len() as f32 / n_possible as f32;

    let above_threshold = ranked.iter().filter(|(_, a)| *a >= train_threshold).count();
    println!(
        "\n── next_block_prediction_accuracy ──\n\
         train_threshold={:.3}\n\
         ranked pairs above threshold: {}\n\
         test-block observed pairs: {} / {} possible (base rate {:.3})",
        train_threshold,
        above_threshold,
        test_pairs.len(),
        n_possible,
        base_rate,
    );

    let candidates: Vec<(u64, u64)> = ranked
        .iter()
        .filter(|(_, a)| *a >= train_threshold)
        .map(|(k, _)| *k)
        .collect();

    for &k in &TOP_KS {
        let top = candidates.iter().take(k).copied().collect::<Vec<_>>();
        if top.is_empty() {
            println!("  k={k}: no candidates above threshold");
            continue;
        }
        let hits = top.iter().filter(|p| test_pairs.contains(p)).count();
        let precision = hits as f32 / top.len() as f32;
        let lift = if base_rate > 0.0 {
            precision / base_rate
        } else {
            0.0
        };
        println!(
            "  precision@{:<3} = {:.3}  (hits {}/{}, lift vs random {:.2}×)",
            k,
            precision,
            hits,
            top.len(),
            lift
        );
    }

    let all_hits = candidates.iter().filter(|p| test_pairs.contains(p)).count();
    let recall = all_hits as f32 / test_pairs.len() as f32;
    println!(
        "  recall (all candidates vs test pairs) = {:.3}  ({} / {})",
        recall,
        all_hits,
        test_pairs.len()
    );

    let k = 20usize.min(candidates.len());
    assert!(
        k > 0,
        "no candidates — train stream produced no above-threshold pairs"
    );
    let top = &candidates[..k];
    let hits = top.iter().filter(|p| test_pairs.contains(p)).count();
    let precision = hits as f32 / top.len() as f32;
    let bar = (2.0 * base_rate).min(0.75);
    assert!(
        precision >= bar,
        "precision@{k} = {precision:.3} below bar {bar:.3} (2× base rate {base_rate:.3} clamped at 0.75); \
         if the signal is this weak, plasticity auto-tuning lacks a clean target"
    );
}

// ── Sanity: generator invariants ─────────────────────────────────────────────

#[test]
fn generator_is_deterministic_and_balanced() {
    let (_w1, _i1, log_a) = run_stream(5, 0xABCD);
    let (_w2, _i2, log_b) = run_stream(5, 0xABCD);
    assert_eq!(
        log_a, log_b,
        "generator must be deterministic under identical seed"
    );

    let mut intra = 0;
    let mut cross = 0;
    for block in &log_a {
        for ev in block {
            for i in 0..ev.len() {
                for j in (i + 1)..ev.len() {
                    if class_of(ev[i]) == class_of(ev[j]) {
                        intra += 1
                    } else {
                        cross += 1
                    }
                }
            }
        }
    }
    assert!(
        intra > cross,
        "intra-class contacts should dominate; got intra={intra} cross={cross}"
    );
}
