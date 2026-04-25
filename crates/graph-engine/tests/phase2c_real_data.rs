//! Phase 2c of the trigger-axis roadmap — real-data measurement.
//!
//! Phase 2b proved `EmergenceThreshold` *can* prune transient evidence on
//! synthetic fixtures. P2c asks the harder question: does it *actually*
//! prune meaningfully on real datasets, and at what cost to the
//! recognize-loop fixpoint?
//!
//! Method: replay a 12-month window of the HEP-PH co-citation dataset
//! twice — once with `EmergenceThreshold::bypass()` (the pre-Phase-2
//! status quo) and once with a non-bypass threshold — and compare:
//!
//!   * total relationships materialised in `RelationshipStore`
//!   * pending entries left in `PreRelationshipBuffer` at run-end
//!   * recognize-loop pass count distribution per checkpoint
//!     (canary against the HEP-PH 2-cycle oscillation finding —
//!      `RECOGNIZE_LAST_PASSES` regression should surface here)
//!   * active entity count
//!   * activity-distribution shift on surviving relationships
//!
//! ## Run
//!
//! ```text
//! cargo test -p graph-engine --test phase2c_real_data -- --ignored --nocapture
//! ```
//!
//! `#[ignore]` because the dataset (~420k edges) makes the run slow and
//! the data file is not in CI.
//!
//! ## What the gate looks like
//!
//! This is a *measurement*, not a pass/fail. The assertions only enforce
//! the structural prediction (threshold reduces relationships) and the
//! safety invariant (recognize fixpoint does not regress). The numerical
//! findings — actual ratio, pass-count delta — are what we record in the
//! roadmap memory and use to decide whether `EmergenceThreshold` ships
//! as opt-in only or with a sensible default.

#![cfg(test)]

use graph_core::{
    ChangeSubject, EntityStatus, InfluenceKindId, Locus, LocusContext, LocusId, LocusKindId,
    LocusProgram, Properties, PropertyValue, ProposedChange, StateVector,
};
use graph_engine::{
    DefaultEmergencePerspective, EmergenceThreshold, Engine, EngineConfig, InfluenceKindConfig,
    InfluenceKindRegistry, LocusKindRegistry, last_recognize_passes,
};
use graph_world::World;
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

const CITES: InfluenceKindId = InfluenceKindId(600);
const PAPER_KIND: LocusKindId = LocusKindId(1);
const RECOGNIZE_EVERY: usize = 3; // checkpoint every 3 months for finer pass-count signal

/// Compact window so the comparison runs in seconds.
fn window_months() -> usize {
    std::env::var("PHASE2C_MONTHS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(12)
}

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data")
}

// ── Data loading (mirrors hep_ph.rs) ──────────────────────────────────────────

fn load_citations(path: &std::path::Path) -> Vec<(u64, u64)> {
    let content = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    content
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .map(|l| {
            let mut it = l.split_whitespace();
            let from: u64 = it.next().unwrap().parse().unwrap();
            let to: u64 = it.next().unwrap().parse().unwrap();
            (from, to)
        })
        .collect()
}

fn load_dates(path: &std::path::Path) -> (HashMap<u64, usize>, usize) {
    let content = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let mut raw: Vec<(u64, u32)> = Vec::new();
    let mut earliest: u32 = u32::MAX;
    for line in content.lines() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let mut it = line.split_whitespace();
        let paper: u64 = it.next().unwrap().parse().unwrap();
        let date = it.next().unwrap();
        let year: u32 = date[0..4].parse().unwrap();
        let month: u32 = date[5..7].parse().unwrap();
        let ym = year * 12 + (month - 1);
        if ym < earliest {
            earliest = ym;
        }
        raw.push((paper, ym));
    }
    let mut map = HashMap::with_capacity(raw.len());
    let mut max_idx: usize = 0;
    for (paper, ym) in raw {
        let idx = (ym - earliest) as usize;
        if idx > max_idx {
            max_idx = idx;
        }
        map.insert(paper, idx);
    }
    (map, max_idx + 1)
}

fn bucket_monthly(
    edges: &[(u64, u64)],
    paper_month: &HashMap<u64, usize>,
    n_months: usize,
) -> Vec<BTreeSet<(u64, u64)>> {
    let mut buckets: Vec<BTreeSet<(u64, u64)>> = vec![BTreeSet::new(); n_months];
    for &(from, to) in edges {
        if from == to {
            continue;
        }
        let Some(&month) = paper_month.get(&from) else {
            continue;
        };
        if month >= n_months {
            continue;
        }
        let pair = if from < to { (from, to) } else { (to, from) };
        buckets[month].insert(pair);
    }
    buckets
}

fn batch_stimuli(pairs: &BTreeSet<(u64, u64)>) -> Vec<ProposedChange> {
    let mut out = Vec::with_capacity(pairs.len() * 2);
    for &(u, v) in pairs {
        let mut meta_u = Properties::new();
        meta_u.set(
            "co_cited",
            PropertyValue::List(vec![PropertyValue::Int(v as i64)]),
        );
        out.push(
            ProposedChange::new(
                ChangeSubject::Locus(LocusId(u)),
                CITES,
                StateVector::from_slice(&[1.0]),
            )
            .with_metadata(meta_u),
        );

        let mut meta_v = Properties::new();
        meta_v.set(
            "co_cited",
            PropertyValue::List(vec![PropertyValue::Int(u as i64)]),
        );
        out.push(
            ProposedChange::new(
                ChangeSubject::Locus(LocusId(v)),
                CITES,
                StateVector::from_slice(&[1.0]),
            )
            .with_metadata(meta_v),
        );
    }
    out
}

struct CitesProgram;

impl LocusProgram for CitesProgram {
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
            let Some(PropertyValue::List(ids)) = meta.get("co_cited") else {
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
                    CITES,
                    StateVector::from_slice(&[1.0]),
                ));
            }
        }
        out
    }
}

// ── Per-scenario run + measurement ───────────────────────────────────────────

#[derive(Debug, Default, Clone)]
struct ScenarioMetrics {
    label: String,
    threshold: EmergenceThreshold,
    relationships_total: usize,
    pre_relationships_total: usize,
    active_entities: usize,
    /// Per-checkpoint recognize_passes counts.
    pass_counts: Vec<usize>,
    /// Distribution of relationship activity magnitudes at run end.
    /// Sorted ascending; useful for percentile reporting.
    activity_distribution: Vec<f32>,
}

impl ScenarioMetrics {
    fn pass_count_summary(&self) -> (usize, usize, f32) {
        if self.pass_counts.is_empty() {
            return (0, 0, 0.0);
        }
        let max = *self.pass_counts.iter().max().unwrap();
        let min = *self.pass_counts.iter().min().unwrap();
        let mean = self.pass_counts.iter().sum::<usize>() as f32 / self.pass_counts.len() as f32;
        (min, max, mean)
    }

    fn activity_percentiles(&self) -> (f32, f32, f32) {
        if self.activity_distribution.is_empty() {
            return (0.0, 0.0, 0.0);
        }
        let n = self.activity_distribution.len();
        let p50 = self.activity_distribution[n / 2];
        let p90 = self.activity_distribution[(n * 9 / 10).min(n - 1)];
        let p99 = self.activity_distribution[(n * 99 / 100).min(n - 1)];
        (p50, p90, p99)
    }
}

fn run_scenario(
    label: &str,
    threshold: EmergenceThreshold,
    nodes: &BTreeSet<u64>,
    buckets: &[BTreeSet<(u64, u64)>],
) -> ScenarioMetrics {
    let mut world = World::new();
    for &node in nodes {
        world.insert_locus(Locus::new(LocusId(node), PAPER_KIND, StateVector::zeros(1)));
    }

    let cfg = InfluenceKindConfig::new("cites")
        .with_decay(0.95)
        .with_symmetric(true)
        .with_emergence_threshold(threshold);
    let mut inf_reg = InfluenceKindRegistry::new();
    inf_reg.insert(CITES, cfg);

    let mut loci_reg = LocusKindRegistry::new();
    loci_reg.insert(PAPER_KIND, Box::new(CitesProgram));

    let engine = Engine::new(EngineConfig {
        max_batches_per_tick: 3,
    });
    let perspective = DefaultEmergencePerspective::default();

    let total_months = buckets.len();
    let mut pass_counts = Vec::new();

    for (month, pairs) in buckets.iter().enumerate() {
        if pairs.is_empty() {
            continue;
        }
        let stimuli = batch_stimuli(pairs);
        engine.tick(&mut world, &loci_reg, &inf_reg, stimuli);

        let is_checkpoint = (month + 1) % RECOGNIZE_EVERY == 0 || month + 1 == total_months;
        if is_checkpoint {
            engine.recognize_entities(&mut world, &inf_reg, &perspective);
            pass_counts.push(last_recognize_passes());
        }
    }

    let active_entities = world
        .entities()
        .iter()
        .filter(|e| e.status == EntityStatus::Active)
        .count();

    let mut activity_distribution: Vec<f32> = world
        .relationships()
        .iter()
        .map(|r| r.activity().abs())
        .collect();
    activity_distribution.sort_by(|a, b| a.partial_cmp(b).unwrap());

    ScenarioMetrics {
        label: label.to_string(),
        threshold,
        relationships_total: world.relationships().len(),
        pre_relationships_total: world.pre_relationships().len(),
        active_entities,
        pass_counts,
        activity_distribution,
    }
}

#[test]
#[ignore = "loads HEP-PH data file (~25 MB); run with --ignored"]
fn phase2c_threshold_measurement_on_hep_ph() {
    let dir = data_dir();
    let citations_path = dir.join("cit-HepPh.txt");
    let dates_path = dir.join("cit-HepPh-dates.txt");
    if !citations_path.exists() || !dates_path.exists() {
        eprintln!(
            "SKIP phase2c: HEP-PH data missing at {} / {}",
            citations_path.display(),
            dates_path.display()
        );
        return;
    }

    let edges = load_citations(&citations_path);
    let (paper_month, n_months_full) = load_dates(&dates_path);
    let window = window_months().min(n_months_full);

    let edges_in_window: Vec<(u64, u64)> = edges
        .iter()
        .copied()
        .filter(|&(from, _)| paper_month.get(&from).map_or(false, |&m| m < window))
        .collect();

    let mut nodes: BTreeSet<u64> = BTreeSet::new();
    for &(u, v) in &edges_in_window {
        nodes.insert(u);
        nodes.insert(v);
    }

    let buckets = bucket_monthly(&edges_in_window, &paper_month, window);
    let total_pairs: usize = buckets.iter().map(|b| b.len()).sum();

    println!("\n═══ Phase 2c HEP-PH Threshold Comparison ═══");
    println!(
        "  window: {}/{} months  edges: {}  nodes: {}  unique pairs: {}",
        window,
        n_months_full,
        edges_in_window.len(),
        nodes.len(),
        total_pairs
    );

    // Response curve: sweep threshold values to expose how pruning rate
    // depends on the parameter. With symmetric kind, one co-citation
    // delivers 2 evidence units, so threshold = 2k means "at least k
    // co-citation events within the window".
    //
    // bypass = pre-Phase-2 behaviour (instant promotion).
    let bypass = run_scenario("bypass", EmergenceThreshold::bypass(), &nodes, &buckets);
    let active_low = run_scenario(
        "threshold(3.0, 12mo) — ≥2 events",
        EmergenceThreshold {
            min_evidence: 3.0,
            window_batches: 12,
        },
        &nodes,
        &buckets,
    );
    let active_mid = run_scenario(
        "threshold(6.0, 12mo) — ≥3 events",
        EmergenceThreshold {
            min_evidence: 6.0,
            window_batches: 12,
        },
        &nodes,
        &buckets,
    );
    let active_high = run_scenario(
        "threshold(10.0, 12mo) — ≥5 events",
        EmergenceThreshold {
            min_evidence: 10.0,
            window_batches: 12,
        },
        &nodes,
        &buckets,
    );

    print_scenario(&bypass);
    print_scenario(&active_low);
    print_scenario(&active_mid);
    print_scenario(&active_high);
    print_comparison(&bypass, &active_low);
    print_comparison(&bypass, &active_mid);
    print_comparison(&bypass, &active_high);

    // Structural prediction: at least the strictest threshold prunes
    // meaningfully. If even threshold=10.0 doesn't bite, the buffer
    // path is bypassed (regression).
    assert!(
        active_high.relationships_total < bypass.relationships_total,
        "even strict threshold should prune at least some pairs \
         (bypass={}, threshold(10.0)={})",
        bypass.relationships_total,
        active_high.relationships_total
    );

    // Safety prediction: threshold must not blow up the recognize fixpoint.
    // Memory records HEP-PH 2-cycle oscillation at the 8-pass cap. If any
    // threshold value pushes pass count beyond bypass + 1, ship-default
    // is unsafe.
    let (_, bypass_max, _) = bypass.pass_count_summary();
    for scenario in [&active_low, &active_mid, &active_high] {
        let (_, active_max, _) = scenario.pass_count_summary();
        assert!(
            active_max <= bypass_max + 1,
            "[{}] threshold regressed recognize-loop convergence \
             (bypass max passes={}, active max passes={})",
            scenario.label,
            bypass_max,
            active_max
        );
    }
}

fn print_scenario(m: &ScenarioMetrics) {
    let (p_min, p_max, p_mean) = m.pass_count_summary();
    let (a50, a90, a99) = m.activity_percentiles();
    println!("\n  [{}]", m.label);
    println!(
        "    threshold: min_evidence={:.2} window={}",
        m.threshold.min_evidence, m.threshold.window_batches
    );
    println!(
        "    relationships materialised:  {:>7}   pending in buffer: {:>7}",
        m.relationships_total, m.pre_relationships_total
    );
    println!("    active entities:             {:>7}", m.active_entities);
    println!(
        "    recognize passes — min={} max={} mean={:.2} (n={})",
        p_min,
        p_max,
        p_mean,
        m.pass_counts.len()
    );
    println!(
        "    activity percentiles — p50={:.3} p90={:.3} p99={:.3}",
        a50, a90, a99
    );
}

fn print_comparison(bypass: &ScenarioMetrics, active: &ScenarioMetrics) {
    let rel_delta = bypass.relationships_total as i64 - active.relationships_total as i64;
    let rel_pct = 100.0 * rel_delta as f64 / bypass.relationships_total.max(1) as f64;
    let buffered = active.pre_relationships_total;
    let promoted = active.relationships_total;
    let total_attempts = buffered + promoted;
    let promote_rate = if total_attempts > 0 {
        100.0 * promoted as f64 / total_attempts as f64
    } else {
        0.0
    };
    let (_, bypass_max, bypass_mean) = bypass.pass_count_summary();
    let (_, active_max, active_mean) = active.pass_count_summary();

    println!("\n  ═══ Comparison ═══");
    println!(
        "    Threshold pruned {} relationships ({:.1}% of bypass count).",
        rel_delta, rel_pct
    );
    println!(
        "    Of {} candidate (key, kind) pairs, {} promoted ({:.1}%) and {} stayed pending.",
        total_attempts, promoted, promote_rate, buffered
    );
    println!(
        "    Recognize-loop passes — bypass max={} mean={:.2}, threshold max={} mean={:.2}",
        bypass_max, bypass_mean, active_max, active_mean
    );

    let entity_delta = bypass.active_entities as i64 - active.active_entities as i64;
    println!(
        "    Active entities — bypass={} threshold={} (Δ={:+})",
        bypass.active_entities, active.active_entities, entity_delta
    );

    println!("\n  ═══ Interpretation ═══");
    if rel_pct >= 25.0 && active_max <= bypass_max {
        println!(
            "    PRUNING IS LOAD-BEARING: threshold removes ≥25% of bypass relationships \
             without regressing the recognize fixpoint. EmergenceThreshold has real value \
             on HEP-PH-shaped data; expose as opt-in with this benchmark cited."
        );
    } else if rel_pct >= 10.0 {
        println!(
            "    PRUNING IS MARGINAL: 10–25% reduction. Worth keeping as an opt-in knob; \
             default = bypass remains the right ship choice."
        );
    } else {
        println!(
            "    PRUNING IS NEGLIGIBLE: <10% reduction. Question whether HEP-PH-shaped \
             accumulative graphs are the right benchmark to validate this knob; try \
             temporal datasets (Enron, EU email) where 1-shot interactions dominate."
        );
    }
}
