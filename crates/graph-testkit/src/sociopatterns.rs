//! Deterministic SocioPatterns-style temporal contact generator and evaluators.
//!
//! This module serves two purposes:
//! - scale the existing `graph-engine/tests/sociopatterns.rs` setup beyond
//!   unit-test size for Criterion benches;
//! - make the Phase 9 "pair prediction" discussion measurable under
//!   different ranking signals (`activity`, `weight`, `strength`).

use std::collections::BTreeSet;

use graph_core::{
    ChangeSubject, Endpoints, InfluenceKindId, Locus, LocusContext, LocusId, LocusKindId,
    LocusProgram, Properties, PropertyValue, ProposedChange, Relationship, StateVector,
};
use graph_engine::{
    Engine, EngineConfig, InfluenceKindConfig, InfluenceKindRegistry, LocusKindRegistry,
    PlasticityConfig,
};
use graph_world::World;

const CO_ATTEND: InfluenceKindId = InfluenceKindId(300);
const STUDENT_KIND: LocusKindId = LocusKindId(1);

#[derive(Debug, Clone, Copy)]
pub struct SocioPatternsProfile {
    pub name: &'static str,
    pub n_students: u64,
    pub class_size: u64,
    pub events_per_block: usize,
    pub group_size_min: usize,
    pub group_size_max: usize,
    pub p_in: f32,
    pub p_out: f32,
    pub p_out_lunch: f32,
    pub lunch_every: usize,
    pub decay: f32,
    pub max_batches_per_tick: u32,
}

impl SocioPatternsProfile {
    pub const fn small() -> Self {
        Self {
            name: "small",
            n_students: 40,
            class_size: 8,
            events_per_block: 10,
            group_size_min: 4,
            group_size_max: 6,
            p_in: 0.75,
            p_out: 0.05,
            p_out_lunch: 0.25,
            lunch_every: 10,
            decay: 0.92,
            max_batches_per_tick: 3,
        }
    }

    pub const fn medium() -> Self {
        Self {
            name: "medium",
            n_students: 120,
            class_size: 12,
            events_per_block: 18,
            group_size_min: 4,
            group_size_max: 6,
            p_in: 0.72,
            p_out: 0.04,
            p_out_lunch: 0.18,
            lunch_every: 10,
            decay: 0.94,
            max_batches_per_tick: 3,
        }
    }

    pub const fn school_scale() -> Self {
        Self {
            name: "school_scale",
            n_students: 240,
            class_size: 24,
            events_per_block: 24,
            group_size_min: 4,
            group_size_max: 7,
            p_in: 0.72,
            p_out: 0.03,
            p_out_lunch: 0.16,
            lunch_every: 10,
            decay: 0.95,
            max_batches_per_tick: 3,
        }
    }

    pub const fn xlarge() -> Self {
        Self {
            name: "xlarge",
            n_students: 480,
            class_size: 24,
            events_per_block: 32,
            group_size_min: 4,
            group_size_max: 7,
            p_in: 0.7,
            p_out: 0.025,
            p_out_lunch: 0.14,
            lunch_every: 10,
            decay: 0.96,
            max_batches_per_tick: 3,
        }
    }

    pub const fn class_count(self) -> u64 {
        self.n_students / self.class_size
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RankSignal {
    Activity,
    Weight,
    Strength,
}

impl RankSignal {
    pub fn label(self) -> &'static str {
        match self {
            Self::Activity => "activity",
            Self::Weight => "weight",
            Self::Strength => "strength",
        }
    }

    fn score(self, rel: &Relationship) -> f32 {
        match self {
            Self::Activity => rel.activity(),
            Self::Weight => rel.weight(),
            Self::Strength => rel.strength(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SocioPatternsRun {
    pub world: World,
    pub event_log: Vec<Vec<Vec<u64>>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PredictionAtK {
    pub k: usize,
    pub hits: usize,
    pub precision: f32,
    pub lift: f32,
}

#[derive(Debug, Clone)]
pub struct PredictionEvaluation {
    pub profile: SocioPatternsProfile,
    pub rank_signal: RankSignal,
    pub train_blocks: usize,
    pub test_blocks: usize,
    pub threshold: f32,
    pub base_rate: f32,
    pub candidate_count: usize,
    pub relationship_count: usize,
    pub test_pair_count: usize,
    pub recall: f32,
    pub ranked_candidates: Vec<(u64, u64)>,
    pub topk: Vec<PredictionAtK>,
}

impl PredictionEvaluation {
    pub fn top_pairs(&self, k: usize) -> Vec<(u64, u64)> {
        self.ranked_candidates.iter().take(k).copied().collect()
    }

    pub fn metric_at(&self, k: usize) -> Option<&PredictionAtK> {
        self.topk.iter().find(|m| m.k == k)
    }
}

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

fn class_of(profile: SocioPatternsProfile, student: u64) -> u64 {
    student / profile.class_size
}

fn class_members(profile: SocioPatternsProfile, class: u64) -> Vec<u64> {
    (class * profile.class_size..(class + 1) * profile.class_size).collect()
}

fn all_classes(profile: SocioPatternsProfile) -> Vec<Vec<u64>> {
    (0..profile.class_count())
        .map(|class| class_members(profile, class))
        .collect()
}

fn sample_event(profile: SocioPatternsProfile, rng: &mut Lcg, block_idx: usize) -> Vec<u64> {
    let lunch = block_idx > 0 && block_idx % profile.lunch_every == 0;
    let p_out = if lunch {
        profile.p_out_lunch
    } else {
        profile.p_out
    };
    let classes = all_classes(profile);

    let focal = rng.next_u64() % profile.n_students;
    let focal_class = class_of(profile, focal);
    let mut attendees = BTreeSet::new();
    attendees.insert(focal);

    let group_size = rng.next_range(profile.group_size_min, profile.group_size_max);
    let mut guard = 0;
    while attendees.len() < group_size && guard < group_size * 8 {
        guard += 1;
        let draw_intra = rng.next_f32() < (profile.p_in / (profile.p_in + p_out));
        let pool = if draw_intra {
            classes[focal_class as usize].as_slice()
        } else {
            let other_class = {
                let mut c = rng.next_u64() % profile.class_count();
                if c == focal_class {
                    c = (c + 1) % profile.class_count();
                }
                c
            };
            classes[other_class as usize].as_slice()
        };
        attendees.insert(rng.choose_in(pool));
    }

    attendees.into_iter().collect()
}

fn block_events(profile: SocioPatternsProfile, rng: &mut Lcg, block_idx: usize) -> Vec<Vec<u64>> {
    (0..profile.events_per_block)
        .map(|_| sample_event(profile, rng, block_idx))
        .collect()
}

fn event_stimuli(attendees: &[u64]) -> Vec<ProposedChange> {
    attendees
        .iter()
        .map(|&student| {
            let others: Vec<PropertyValue> = attendees
                .iter()
                .filter(|&&x| x != student)
                .map(|&id| PropertyValue::Int(id as i64))
                .collect();
            let mut meta = Properties::new();
            meta.set("co_attendees", PropertyValue::List(others));
            ProposedChange::new(
                ChangeSubject::Locus(LocusId(student)),
                CO_ATTEND,
                StateVector::from_slice(&[1.0]),
            )
            .with_metadata(meta)
        })
        .collect()
}

fn insert_students(world: &mut World, profile: SocioPatternsProfile) {
    for student in 0..profile.n_students {
        world.insert_locus(Locus::new(
            LocusId(student),
            STUDENT_KIND,
            StateVector::zeros(1),
        ));
    }
}

fn make_inf_reg(
    profile: SocioPatternsProfile,
    plasticity: PlasticityConfig,
) -> InfluenceKindRegistry {
    let mut cfg = InfluenceKindConfig::new("co_attendance")
        .with_decay(profile.decay)
        .with_symmetric(true);
    if plasticity.learning_rate > 0.0 {
        cfg = cfg.with_plasticity(plasticity);
    }
    let mut reg = InfluenceKindRegistry::new();
    reg.insert(CO_ATTEND, cfg);
    reg
}

fn make_loci_reg() -> LocusKindRegistry {
    let mut reg = LocusKindRegistry::new();
    reg.insert(STUDENT_KIND, Box::new(CoAttendanceProgram));
    reg
}

pub fn diagnostic_threshold_from_scores(mut scores: Vec<f32>) -> f32 {
    scores.retain(|score| *score > 0.0);
    if scores.len() < 4 {
        return 0.0;
    }
    scores.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let search_end = scores.len() * 3 / 4 + 1;
    let mut best_gap_ratio = 1.0f32;
    let mut best_threshold = 0.0f32;
    for i in 0..search_end.saturating_sub(1) {
        let a = scores[i];
        let b = scores[i + 1];
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

pub fn diagnostic_threshold(world: &World, rank_signal: RankSignal) -> f32 {
    diagnostic_threshold_from_scores(
        world
            .relationships()
            .iter()
            .map(|rel| rank_signal.score(rel))
            .collect(),
    )
}

pub fn run_stream(
    profile: SocioPatternsProfile,
    blocks_to_run: usize,
    seed: u64,
    plasticity: PlasticityConfig,
) -> SocioPatternsRun {
    assert!(
        profile.n_students % profile.class_size == 0,
        "n_students must be divisible by class_size"
    );

    let mut world = World::new();
    insert_students(&mut world, profile);
    let inf = make_inf_reg(profile, plasticity);
    let loci_reg = make_loci_reg();
    let engine = Engine::new(EngineConfig {
        max_batches_per_tick: profile.max_batches_per_tick,
    });
    let mut rng = Lcg::new(seed);

    let mut event_log = Vec::with_capacity(blocks_to_run);
    for block_idx in 0..blocks_to_run {
        let events = block_events(profile, &mut rng, block_idx);
        let mut stimuli = Vec::new();
        for event in &events {
            if event.len() >= 2 {
                stimuli.extend(event_stimuli(event));
            }
        }
        if !stimuli.is_empty() {
            engine.tick(&mut world, &loci_reg, &inf, stimuli);
        }
        event_log.push(events);
    }

    SocioPatternsRun { world, event_log }
}

pub fn evaluate_next_block_prediction(
    profile: SocioPatternsProfile,
    seed: u64,
    train_blocks: usize,
    test_blocks: usize,
    plasticity: PlasticityConfig,
    rank_signal: RankSignal,
    top_ks: &[usize],
) -> PredictionEvaluation {
    let full_run = run_stream(profile, train_blocks + test_blocks, seed, plasticity);
    let train_run = run_stream(profile, train_blocks, seed, plasticity);

    let threshold = diagnostic_threshold(&train_run.world, rank_signal);
    let ranked_candidates = ranked_candidates(&train_run.world, rank_signal, threshold);
    let test_pairs = test_pairs_from_blocks(&full_run.event_log[train_blocks..]);
    let base_rate = base_pair_rate(profile, test_pairs.len());
    let topk = evaluate_topk(&ranked_candidates, &test_pairs, base_rate, top_ks);
    let recall = recall_against_pairs(&ranked_candidates, &test_pairs);

    PredictionEvaluation {
        profile,
        rank_signal,
        train_blocks,
        test_blocks,
        threshold,
        base_rate,
        candidate_count: ranked_candidates.len(),
        relationship_count: train_run.world.relationships().len(),
        test_pair_count: test_pairs.len(),
        recall,
        ranked_candidates,
        topk,
    }
}

fn ranked_candidates(
    world: &graph_world::World,
    rank_signal: RankSignal,
    threshold: f32,
) -> Vec<(u64, u64)> {
    let mut ranked: Vec<((u64, u64), f32)> = world
        .relationships()
        .iter()
        .filter_map(|rel| symmetric_ranked_pair(rel, rank_signal))
        .collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    ranked
        .into_iter()
        .filter(|(_, score)| *score >= threshold)
        .map(|(pair, _)| pair)
        .collect()
}

fn symmetric_ranked_pair(
    rel: &graph_core::Relationship,
    rank_signal: RankSignal,
) -> Option<((u64, u64), f32)> {
    match rel.endpoints {
        Endpoints::Symmetric { a, b } => Some((ordered_pair(a.0, b.0), rank_signal.score(rel))),
        _ => None,
    }
}

fn test_pairs_from_blocks(blocks: &[Vec<Vec<u64>>]) -> BTreeSet<(u64, u64)> {
    let mut test_pairs = BTreeSet::new();
    for block in blocks {
        for event in block {
            for i in 0..event.len() {
                for j in (i + 1)..event.len() {
                    test_pairs.insert(ordered_pair(event[i], event[j]));
                }
            }
        }
    }
    test_pairs
}

fn ordered_pair(a: u64, b: u64) -> (u64, u64) {
    if a < b { (a, b) } else { (b, a) }
}

fn base_pair_rate(profile: SocioPatternsProfile, test_pair_count: usize) -> f32 {
    let possible_pairs = (profile.n_students * (profile.n_students - 1) / 2) as usize;
    test_pair_count as f32 / possible_pairs as f32
}

fn evaluate_topk(
    ranked_candidates: &[(u64, u64)],
    test_pairs: &BTreeSet<(u64, u64)>,
    base_rate: f32,
    top_ks: &[usize],
) -> Vec<PredictionAtK> {
    top_ks
        .iter()
        .copied()
        .filter(|k| *k > 0)
        .map(|k| prediction_at_k(ranked_candidates, test_pairs, base_rate, k))
        .collect()
}

fn prediction_at_k(
    ranked_candidates: &[(u64, u64)],
    test_pairs: &BTreeSet<(u64, u64)>,
    base_rate: f32,
    k: usize,
) -> PredictionAtK {
    let top = ranked_candidates
        .iter()
        .take(k)
        .copied()
        .collect::<Vec<_>>();
    let hits = top.iter().filter(|pair| test_pairs.contains(pair)).count();
    let precision = if top.is_empty() {
        0.0
    } else {
        hits as f32 / top.len() as f32
    };
    let lift = if base_rate > 0.0 {
        precision / base_rate
    } else {
        0.0
    };
    PredictionAtK {
        k,
        hits,
        precision,
        lift,
    }
}

fn recall_against_pairs(
    ranked_candidates: &[(u64, u64)],
    test_pairs: &BTreeSet<(u64, u64)>,
) -> f32 {
    let all_hits = ranked_candidates
        .iter()
        .filter(|pair| test_pairs.contains(pair))
        .count();
    if test_pairs.is_empty() {
        0.0
    } else {
        all_hits as f32 / test_pairs.len() as f32
    }
}
