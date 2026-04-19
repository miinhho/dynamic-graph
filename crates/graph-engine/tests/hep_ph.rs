//! Real-data oracle test: ArXiv HEP-PH citation network (SNAP dataset).
//!
//! Contrast with `eu_email.rs`. EU email exposed a **dynamic-temporal**
//! failure mode (community membership churns weekly → Born flood). HEP-PH
//! is the opposite end of the contract: citations are **accumulative**
//! (once paper A cites paper B, that link exists forever), so community
//! structure (physics subfields) is expected to evolve gradually. This
//! test checks whether the engine excels on its designed contract at
//! a scale 35× larger than EU email.
//!
//! Data files (relative to workspace root):
//!   `data/cit-HepPh.txt`         — 421,578 citation edges, format: `fromPaper toPaper`
//!   `data/cit-HepPh-dates.txt`   — 38,557 paper dates,   format: `paperId YYYY-MM-DD`
//!
//! Run with:
//!   cargo test -p graph-engine --test hep_ph -- --ignored --nocapture
//!
//! ## What this test measures
//!
//! 1. **Born/Dormant ratio** on an accumulative temporal graph. EU email
//!    produced Born=20,484 vs Dormant=1 (event count), a dataset-property
//!    explosion. HEP-PH should be closer to parity if the engine's
//!    gradual-evolution contract holds.
//! 2. **Active entity / paper ratio**. EU email reached 14,624 active
//!    entities from 986 people (14.8×). Engineered expectation for HEP-PH:
//!    ≤5× (~170K ceiling) — hopefully far lower.
//! 3. **Scale behaviour at ~34K nodes / 420K edges / 122 monthly batches**.
//!    First real workload outside the 120–986-node regime.
//!
//! Ground truth subfield labels are NOT present in this SNAP release, so
//! NMI is not computed. Discovery run: no precision assertions yet.

use graph_core::{
    ChangeSubject, EntityStatus, InfluenceKindId, LayerTransition, Locus, LocusContext,
    LocusId, LocusKindId, LocusProgram, Properties, PropertyValue, ProposedChange, StateVector,
};
use graph_engine::{
    DefaultEmergencePerspective, Engine, EngineConfig, InfluenceKindConfig, InfluenceKindRegistry,
    LocusKindRegistry, debug_exclusivity_counters, debug_last_component_count,
    last_recognize_passes, reset_exclusivity_counters,
};
use graph_world::World;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;

const CITES: InfluenceKindId = InfluenceKindId(600);
const PAPER_KIND: LocusKindId = LocusKindId(1);
const RECOGNIZE_EVERY: usize = 6; // checkpoint every 6 months

/// Scale guard. Override with `HEP_PH_MAX_MONTHS=N` env var. Full dataset is
/// 122 months (1992-02 ~ 2002-03). Default 24 months keeps memory usage
/// bounded for local machines: larger windows accumulate hundreds of
/// thousands of entities × layers × relationships and can exceed 8GB RSS.
fn max_months() -> usize {
    std::env::var("HEP_PH_MAX_MONTHS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(24)
}

/// Safety cap on active entity count. If exceeded at any checkpoint the
/// run aborts early to avoid OOM. Override with `HEP_PH_MAX_ENTITIES=N`.
fn max_entities() -> usize {
    std::env::var("HEP_PH_MAX_ENTITIES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(50_000)
}

// ── LocusProgram ──────────────────────────────────────────────────────────────

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
            let Some(meta) = change.metadata.as_ref() else { continue };
            let Some(PropertyValue::List(ids)) = meta.get("co_cited") else { continue };
            for val in ids {
                let PropertyValue::Int(id) = val else { continue };
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

// ── Data loading ──────────────────────────────────────────────────────────────

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data")
}

/// Load citation edges (fromPaper, toPaper). Skips comment lines.
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

/// Load paper → month_index. Month index is 0-based from earliest date.
/// Returns (paper → month_idx, total_months).
fn load_dates(path: &std::path::Path) -> (HashMap<u64, usize>, usize) {
    let content = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));

    // First pass: collect (paper, (year, month))
    let mut raw: Vec<(u64, u32)> = Vec::new();
    let mut earliest: u32 = u32::MAX;
    for line in content.lines() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let mut it = line.split_whitespace();
        let paper: u64 = it.next().unwrap().parse().unwrap();
        let date = it.next().unwrap();
        // YYYY-MM-DD → year*12 + month
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

// ── Batch bucketing ───────────────────────────────────────────────────────────

/// Group citation edges into monthly buckets by *citing-paper publication month*.
/// Edges whose citing paper has no date entry are dropped. Self-citations dropped.
/// Pairs within a month are deduplicated (unordered).
fn bucket_monthly(
    edges: &[(u64, u64)],
    paper_month: &HashMap<u64, usize>,
    n_months: usize,
) -> (Vec<BTreeSet<(u64, u64)>>, usize, usize) {
    let mut buckets: Vec<BTreeSet<(u64, u64)>> = vec![BTreeSet::new(); n_months];
    let mut dropped_no_date = 0usize;
    let mut dropped_self = 0usize;
    for &(from, to) in edges {
        if from == to {
            dropped_self += 1;
            continue;
        }
        let Some(&month) = paper_month.get(&from) else {
            dropped_no_date += 1;
            continue;
        };
        let pair = if from < to { (from, to) } else { (to, from) };
        buckets[month].insert(pair);
    }
    (buckets, dropped_no_date, dropped_self)
}

// ── Stimuli ───────────────────────────────────────────────────────────────────

fn batch_stimuli(pairs: &BTreeSet<(u64, u64)>) -> Vec<ProposedChange> {
    let mut out = Vec::with_capacity(pairs.len() * 2);
    for &(u, v) in pairs {
        let mut meta_u = Properties::new();
        meta_u.set("co_cited", PropertyValue::List(vec![PropertyValue::Int(v as i64)]));
        out.push(
            ProposedChange::new(
                ChangeSubject::Locus(LocusId(u)),
                CITES,
                StateVector::from_slice(&[1.0]),
            )
            .with_metadata(meta_u),
        );

        let mut meta_v = Properties::new();
        meta_v.set("co_cited", PropertyValue::List(vec![PropertyValue::Int(u as i64)]));
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

// ── Lifecycle counting ────────────────────────────────────────────────────────

#[derive(Debug, Default)]
struct LifecycleCounts {
    born: usize,
    split: usize,
    merge: usize,
    dormant: usize,
    revived: usize,
    membership_delta: usize,
    coherence_shift: usize,
}

fn count_lifecycle(world: &World) -> LifecycleCounts {
    let mut c = LifecycleCounts::default();
    for e in world.entities().iter() {
        for layer in &e.layers {
            match &layer.transition {
                LayerTransition::Born => c.born += 1,
                LayerTransition::Split { .. } => c.split += 1,
                LayerTransition::Merged { .. } => c.merge += 1,
                LayerTransition::BecameDormant => c.dormant += 1,
                LayerTransition::Revived => c.revived += 1,
                LayerTransition::MembershipDelta { .. } => c.membership_delta += 1,
                LayerTransition::CoherenceShift { .. } => c.coherence_shift += 1,
            }
        }
    }
    c
}

// ── Main test ─────────────────────────────────────────────────────────────────

fn run_scenario(label: &str, decay: f32, threshold: Option<f32>) {
    reset_exclusivity_counters();
    let dir = data_dir();
    let edges = load_citations(&dir.join("cit-HepPh.txt"));
    let (paper_month, n_months_full) = load_dates(&dir.join("cit-HepPh-dates.txt"));
    let window = max_months().min(n_months_full);

    // Filter edges to citing-paper month < window. Also derive node set from
    // the filtered edges only — loci for papers outside the window are not
    // created. This keeps both the edge count and locus count bounded.
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
    let n_nodes = nodes.len();

    let (buckets, dropped_no_date, dropped_self) =
        bucket_monthly(&edges_in_window, &paper_month, window);
    let nonempty = buckets.iter().filter(|b| !b.is_empty()).count();
    let total_pairs: usize = buckets.iter().map(|b| b.len()).sum();

    println!("\n═══ HEP-PH Oracle [{}] ═══", label);
    println!(
        "  window: {}/{} months  edges_total: {}  edges_in_window: {}  nodes: {}  dated: {}",
        window,
        n_months_full,
        edges.len(),
        edges_in_window.len(),
        n_nodes,
        paper_month.len(),
    );
    println!(
        "  decay: {}  threshold: {:?}  max_entities_guard: {}",
        decay,
        threshold,
        max_entities()
    );
    println!(
        "  monthly batches: {}  non-empty: {}  total unique pairs: {}  avg/month: {:.0}",
        buckets.len(),
        nonempty,
        total_pairs,
        if nonempty > 0 { total_pairs as f64 / nonempty as f64 } else { 0.0 },
    );
    println!(
        "  dropped edges — no-date citer: {}  self-citation: {}",
        dropped_no_date, dropped_self
    );

    // Build world: only instantiate loci that actually appear in some edge.
    let mut world = World::new();
    for &node in &nodes {
        world.insert_locus(Locus::new(LocusId(node), PAPER_KIND, StateVector::zeros(1)));
    }

    let cfg = InfluenceKindConfig::new("cites")
        .with_decay(decay)
        .with_symmetric(true);
    let mut inf_reg = InfluenceKindRegistry::new();
    inf_reg.insert(CITES, cfg);

    let mut loci_reg = LocusKindRegistry::new();
    loci_reg.insert(PAPER_KIND, Box::new(CitesProgram));

    let engine = Engine::new(EngineConfig { max_batches_per_tick: 3 });
    let perspective = match threshold {
        None => DefaultEmergencePerspective::default(),
        Some(t) => DefaultEmergencePerspective::default().with_min_activity_threshold(t),
    };

    let total_months = buckets.len();
    // (month, active_entities, relationship_count, median_activity, component_count, idempotent_delta)
    let mut snapshots: Vec<(usize, usize, usize, f32, usize, i64)> = Vec::new();

    for (month, pairs) in buckets.iter().enumerate() {
        if pairs.is_empty() {
            continue;
        }
        let stimuli = batch_stimuli(pairs);
        engine.tick(&mut world, &loci_reg, &inf_reg, stimuli);

        let is_checkpoint = (month + 1) % RECOGNIZE_EVERY == 0 || month + 1 == total_months;
        if is_checkpoint {
            engine.recognize_entities(&mut world, &inf_reg, &perspective);
            let n_components_first = debug_last_component_count();
            let passes_first = last_recognize_passes();
            let active_first = world
                .entities()
                .iter()
                .filter(|e| e.status == EntityStatus::Active)
                .count();

            // Idempotency probe: recognize again without stimuli. With the
            // fixpoint wrapper in place, this should now be a no-op (proposals
            // empty on the first pass → single pass).
            engine.recognize_entities(&mut world, &inf_reg, &perspective);
            let passes_second = last_recognize_passes();
            let active_second = world
                .entities()
                .iter()
                .filter(|e| e.status == EntityStatus::Active)
                .count();
            let idempotent_delta = active_second as i64 - active_first as i64;
            let _ = passes_second; // inspection only
            let _ = passes_first;

            let rel_count = world.relationships().iter().count();
            let mut acts: Vec<f32> = world
                .relationships()
                .iter()
                .map(|r| r.state.as_slice()[0].abs())
                .filter(|&a| a > 0.0)
                .collect();
            acts.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let median_act = acts.get(acts.len() / 2).copied().unwrap_or(0.0);
            let active = active_second;
            snapshots.push((
                month + 1,
                active,
                rel_count,
                median_act,
                n_components_first,
                idempotent_delta,
            ));
            println!(
                "  [month {:>3}] active={:>7} comps={:>7} rels={:>7} median_act={:.4} passes1={} passes2={} Δidempotent={:+}",
                month + 1,
                active,
                n_components_first,
                rel_count,
                median_act,
                passes_first,
                passes_second,
                idempotent_delta
            );
            let cap = max_entities();
            if active > cap {
                println!(
                    "  !! ABORT at month {}: active {} > guard {} (set HEP_PH_MAX_ENTITIES to raise)",
                    month + 1,
                    active,
                    cap
                );
                break;
            }
        }
    }

    let active_entities: Vec<_> = world
        .entities()
        .iter()
        .filter(|e| e.status == EntityStatus::Active)
        .collect();
    let n_active = active_entities.len();

    // Entity-size distribution
    let sizes: Vec<usize> = active_entities.iter().map(|e| e.current.members.len()).collect();
    let mut size_hist: BTreeMap<usize, usize> = BTreeMap::new();
    for &s in &sizes {
        *size_hist.entry(s).or_insert(0) += 1;
    }
    let total_members_in_entities: usize = sizes.iter().sum();
    let max_size = sizes.iter().copied().max().unwrap_or(0);
    let median_size = if sizes.is_empty() {
        0
    } else {
        let mut s = sizes.clone();
        s.sort_unstable();
        s[s.len() / 2]
    };

    let lc = count_lifecycle(&world);

    println!("\n── Entity checkpoints (monthly) ──");
    println!(
        "  {:>5}  {:>8}  {:>7}  {:>9}  {:>10}  {:>6}",
        "month", "active", "comps", "rels", "median_act", "Δidem"
    );
    for &(month, count, rels, med, comps, delta) in &snapshots {
        let flag = if count > n_nodes * 5 { "  !! explosion" } else { "" };
        println!(
            "  {:>5}  {:>8}  {:>7}  {:>9}  {:>10.4}  {:>+6}{}",
            month, count, comps, rels, med, delta, flag
        );
    }

    println!("\n── Final snapshot ──");
    println!(
        "  Active entities: {}  (nodes: {}  ratio: {:.2}×)",
        n_active, n_nodes, n_active as f64 / n_nodes as f64
    );
    println!(
        "  Entity members: total={} median_size={} max_size={}",
        total_members_in_entities, median_size, max_size
    );

    let (excl_unchanged, excl_filtered, excl_collapsed) = debug_exclusivity_counters();
    println!("\n── Exclusivity filter trips (Born path) ──");
    println!(
        "  unchanged={} filtered={} collapsed={} (ratio filtered+collapsed = {:.1}%)",
        excl_unchanged,
        excl_filtered,
        excl_collapsed,
        if excl_unchanged + excl_filtered + excl_collapsed > 0 {
            100.0 * (excl_filtered + excl_collapsed) as f64
                / (excl_unchanged + excl_filtered + excl_collapsed) as f64
        } else {
            0.0
        }
    );

    println!("\n── Lifecycle events (all time) ──");
    println!("  Born:            {}", lc.born);
    println!("  Split:           {}", lc.split);
    println!("  Merge:           {}", lc.merge);
    println!("  BecameDormant:   {}", lc.dormant);
    println!("  Revived:         {}", lc.revived);
    println!("  MembershipDelta: {}", lc.membership_delta);
    println!("  CoherenceShift:  {}", lc.coherence_shift);
    println!(
        "  Born/Split:      {:.2}×  Born/(Split+Dormant): {:.2}×",
        if lc.split > 0 { lc.born as f64 / lc.split as f64 } else { f64::INFINITY },
        if lc.split + lc.dormant > 0 {
            lc.born as f64 / (lc.split + lc.dormant) as f64
        } else {
            f64::INFINITY
        },
    );

    assert!(lc.born >= 1, "no entities emerged from {} citations", edges.len());
}

/// Primary oracle run: DECAY=0.9, auto-threshold. Matches the EU email
/// `slow_decay` configuration so the two can be compared head-to-head.
#[test]
#[ignore = "requires external data: data/cit-HepPh.txt, cit-HepPh-dates.txt"]
fn hep_ph_slow_decay_auto() {
    run_scenario("slow_decay_auto", 0.9, None);
}

/// Fast-decay sanity check. With accumulative citation data, DECAY=0.5
/// (half-life 1 month) should give lower active-entity persistence than
/// 0.9 but must not collapse to zero the way EU email did (EU email's
/// collapse was driven by *sporadic* recurrence; HEP-PH citations arrive
/// continuously).
#[test]
#[ignore = "requires external data: data/cit-HepPh.txt, cit-HepPh-dates.txt"]
fn hep_ph_fast_decay_auto() {
    run_scenario("fast_decay_auto", 0.5, None);
}

/// Very slow decay: DECAY=0.98 (half-life ~34 months). Accumulative ceiling
/// test — citation activity should saturate and produce the most stable
/// community structure. If the engine's gradual-evolution contract holds,
/// this run should show the lowest Born rate and smallest active/node ratio.
#[test]
#[ignore = "requires external data: data/cit-HepPh.txt, cit-HepPh-dates.txt"]
fn hep_ph_very_slow_decay_auto() {
    run_scenario("very_slow_decay_auto", 0.98, None);
}
