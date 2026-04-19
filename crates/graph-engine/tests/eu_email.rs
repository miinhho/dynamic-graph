//! Real-data oracle test: EU email temporal network (SNAP dataset).
//!
//! Data files (relative to workspace root):
//!   `data/email-Eu-core-temporal.txt`  — 332,334 directed edges, format: `from to seconds`
//!   `data/email-Eu-core-dept-labels.txt` — 986 nodes, 42 departments, format: `node dept`
//!
//! Run with:
//!   cargo test -p graph-engine --test eu_email -- --ignored --nocapture
//!
//! ## What this test measures
//!
//! 1. **Auto-threshold on real data** — does `min_activity_threshold: None` (gap
//!    detector) survive a real email distribution that is not guaranteed to be
//!    bimodal?
//! 2. **Lifecycle counts** — how many Born / Dormant / Revived events emerge
//!    from 115 weeks of real communication data?
//! 3. **Partition quality** — NMI between engine-derived entities and the 42
//!    ground-truth departments.
//!
//! This is a discovery test. It prints results and makes only one loose assertion
//! (at least one entity must emerge). Threshold assertions will be added once
//! baseline numbers are known.

use graph_core::{
    ChangeSubject, EntityStatus, InfluenceKindId, LayerTransition, Locus, LocusContext,
    LocusId, LocusKindId, LocusProgram, Properties, PropertyValue, ProposedChange, StateVector,
};
use graph_engine::{
    DefaultEmergencePerspective, Engine, EngineConfig, InfluenceKindConfig, InfluenceKindRegistry,
    LocusKindRegistry,
};
use graph_world::World;
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

const EMAIL: InfluenceKindId = InfluenceKindId(500);
const PERSON_KIND: LocusKindId = LocusKindId(1);
const WEEK_SECS: u64 = 7 * 24 * 3600; // 604,800 seconds
const DECAY: f32 = 0.5;
const RECOGNIZE_EVERY: usize = 10;

// ── LocusProgram ──────────────────────────────────────────────────────────────

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
            let Some(meta) = change.metadata.as_ref() else { continue };
            let Some(PropertyValue::List(ids)) = meta.get("co_members") else { continue };
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

// ── Data loading ──────────────────────────────────────────────────────────────

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data")
}

fn load_edges(path: &std::path::Path) -> Vec<(u64, u64, u64)> {
    let content = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let mut it = l.split_whitespace();
            let from: u64 = it.next().unwrap().parse().unwrap();
            let to: u64 = it.next().unwrap().parse().unwrap();
            let ts: u64 = it.next().unwrap().parse().unwrap();
            (from, to, ts)
        })
        .collect()
}

fn load_labels(path: &std::path::Path) -> HashMap<u64, u64> {
    let content = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let mut it = l.split_whitespace();
            let node: u64 = it.next().unwrap().parse().unwrap();
            let dept: u64 = it.next().unwrap().parse().unwrap();
            (node, dept)
        })
        .collect()
}

// ── Batch bucketing ───────────────────────────────────────────────────────────

/// Group directed email edges into weekly buckets.
/// Self-loops are dropped; pairs are deduplicated per week (unordered).
fn bucket_weekly(edges: &[(u64, u64, u64)]) -> Vec<BTreeSet<(u64, u64)>> {
    let max_ts = edges.iter().map(|&(_, _, ts)| ts).max().unwrap_or(0);
    let n_weeks = (max_ts / WEEK_SECS + 1) as usize;
    let mut buckets: Vec<BTreeSet<(u64, u64)>> = vec![BTreeSet::new(); n_weeks];
    for &(from, to, ts) in edges {
        if from == to {
            continue;
        }
        let week = (ts / WEEK_SECS) as usize;
        let pair = if from < to { (from, to) } else { (to, from) };
        buckets[week].insert(pair);
    }
    buckets
}

// ── Stimuli ───────────────────────────────────────────────────────────────────

/// For each unique pair (u, v) in this week's batch, fire a stimulus on
/// both endpoints — same co-activation pattern used in enron.rs.
fn batch_stimuli(pairs: &BTreeSet<(u64, u64)>) -> Vec<ProposedChange> {
    let mut out = Vec::with_capacity(pairs.len() * 2);
    for &(u, v) in pairs {
        let mut meta_u = Properties::new();
        meta_u.set("co_members", PropertyValue::List(vec![PropertyValue::Int(v as i64)]));
        out.push(
            ProposedChange::new(
                ChangeSubject::Locus(LocusId(u)),
                EMAIL,
                StateVector::from_slice(&[1.0]),
            )
            .with_metadata(meta_u),
        );

        let mut meta_v = Properties::new();
        meta_v.set("co_members", PropertyValue::List(vec![PropertyValue::Int(u as i64)]));
        out.push(
            ProposedChange::new(
                ChangeSubject::Locus(LocusId(v)),
                EMAIL,
                StateVector::from_slice(&[1.0]),
            )
            .with_metadata(meta_v),
        );
    }
    out
}

// ── NMI ──────────────────────────────────────────────────────────────────────

/// Normalised Mutual Information. Both slices are 0-based dense cluster indices,
/// same length. `n_pred` / `n_truth` are the cluster count upper bounds.
fn nmi_from_assignments(pred: &[usize], truth: &[usize], n_pred: usize, n_truth: usize) -> f64 {
    let n = pred.len() as f64;
    if n == 0.0 || n_pred == 0 || n_truth == 0 {
        return 0.0;
    }

    let mut contingency = vec![vec![0usize; n_truth]; n_pred];
    for (&p, &t) in pred.iter().zip(truth.iter()) {
        contingency[p][t] += 1;
    }

    let row_sums: Vec<usize> = contingency.iter().map(|r| r.iter().sum()).collect();
    let col_sums: Vec<usize> = (0..n_truth)
        .map(|j| (0..n_pred).map(|i| contingency[i][j]).sum())
        .collect();

    let mut mi = 0.0f64;
    for i in 0..n_pred {
        for j in 0..n_truth {
            let nij = contingency[i][j] as f64;
            if nij == 0.0 {
                continue;
            }
            mi += (nij / n) * (n * nij / (row_sums[i] as f64 * col_sums[j] as f64)).ln();
        }
    }

    let entropy = |sums: &[usize]| -> f64 {
        sums.iter().fold(0.0, |acc, &s| {
            let p = s as f64 / n;
            if p > 0.0 { acc - p * p.ln() } else { acc }
        })
    };
    let h_pred = entropy(&row_sums);
    let h_truth = entropy(&col_sums);

    if h_pred + h_truth == 0.0 {
        0.0
    } else {
        2.0 * mi / (h_pred + h_truth)
    }
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

#[test]
#[ignore = "requires external data: data/email-Eu-core-temporal.txt"]
fn eu_email_temporal_partition_quality() {
    let dir = data_dir();
    let edges = load_edges(&dir.join("email-Eu-core-temporal.txt"));
    let labels = load_labels(&dir.join("email-Eu-core-dept-labels.txt"));

    // Unique nodes from edge list
    let mut nodes: BTreeSet<u64> = BTreeSet::new();
    for &(u, v, _) in &edges {
        nodes.insert(u);
        nodes.insert(v);
    }
    let n_nodes = nodes.len();
    let unique_depts: BTreeSet<u64> = labels.values().copied().collect();

    println!("\n═══ EU Email Temporal Oracle ═══");
    println!(
        "  edges: {}  nodes: {}  labelled nodes: {}  unique depts: {}",
        edges.len(),
        n_nodes,
        labels.len(),
        unique_depts.len()
    );

    // Weekly buckets
    let buckets = bucket_weekly(&edges);
    let nonempty = buckets.iter().filter(|b| !b.is_empty()).count();
    let total_pairs: usize = buckets.iter().map(|b| b.len()).sum();
    println!(
        "  weekly batches: {}  non-empty: {}  total unique pairs: {}  avg/week: {:.0}",
        buckets.len(),
        nonempty,
        total_pairs,
        total_pairs as f64 / nonempty as f64,
    );

    // Build world
    let mut world = World::new();
    for &node in &nodes {
        world.insert_locus(Locus::new(LocusId(node), PERSON_KIND, StateVector::zeros(1)));
    }

    let cfg = InfluenceKindConfig::new("email")
        .with_decay(DECAY)
        .with_symmetric(true);
    let mut inf_reg = InfluenceKindRegistry::new();
    inf_reg.insert(EMAIL, cfg);

    let mut loci_reg = LocusKindRegistry::new();
    loci_reg.insert(PERSON_KIND, Box::new(EmailProgram));

    let engine = Engine::new(EngineConfig { max_batches_per_tick: 3 });
    let perspective = DefaultEmergencePerspective::default();

    // Stream weekly batches
    // (week, active_entities, relationship_count, median_activity)
    let mut entity_snapshots: Vec<(usize, usize, usize, f32)> = Vec::new();
    let total_weeks = buckets.len();

    for (week, pairs) in buckets.iter().enumerate() {
        if pairs.is_empty() {
            continue;
        }
        let stimuli = batch_stimuli(pairs);
        engine.tick(&mut world, &loci_reg, &inf_reg, stimuli);

        let is_checkpoint = (week + 1) % RECOGNIZE_EVERY == 0 || week + 1 == total_weeks;
        if is_checkpoint {
            engine.recognize_entities(&mut world, &inf_reg, &perspective);
            let active = world
                .entities()
                .iter()
                .filter(|e| e.status == EntityStatus::Active)
                .count();
            let rel_count = world.relationships().iter().count();
            let mut acts: Vec<f32> = world
                .relationships()
                .iter()
                .map(|r| r.state.as_slice()[0].abs())
                .filter(|&a| a > 0.0)
                .collect();
            acts.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let median_act = acts.get(acts.len() / 2).copied().unwrap_or(0.0);
            if active > n_nodes * 5 {
                println!(
                    "  WARNING week {:>3}: entity explosion — {} active > 5×{} nodes  rels={} median_act={:.4}",
                    week + 1, active, n_nodes, rel_count, median_act
                );
            }
            entity_snapshots.push((week + 1, active, rel_count, median_act));
        }
    }

    // Final entity snapshot and partition quality
    let lc = count_lifecycle(&world);
    let active_entities: Vec<_> = world
        .entities()
        .iter()
        .filter(|e| e.status == EntityStatus::Active)
        .collect();
    let n_active = active_entities.len();

    // node → entity cluster index
    let mut node_to_cluster: HashMap<u64, usize> = HashMap::new();
    for (ci, ent) in active_entities.iter().enumerate() {
        for &locus_id in &ent.current.members {
            node_to_cluster.insert(locus_id.0, ci);
        }
    }

    // Build parallel pred/truth arrays for nodes that have both a label and
    // an entity assignment.
    let mut pred_labels: Vec<usize> = Vec::new();
    let mut truth_labels: Vec<usize> = Vec::new();
    let mut dept_index: HashMap<u64, usize> = HashMap::new();

    for (&node, &dept) in &labels {
        if let Some(&cluster) = node_to_cluster.get(&node) {
            pred_labels.push(cluster);
            let next = dept_index.len();
            let idx = *dept_index.entry(dept).or_insert(next);
            truth_labels.push(idx);
        }
    }
    let n_depts_observed = dept_index.len();

    let nmi_score = if pred_labels.is_empty() {
        0.0
    } else {
        nmi_from_assignments(&pred_labels, &truth_labels, n_active, n_depts_observed)
    };

    // Report
    println!("\n── Entity count checkpoints ──");
    println!("  {:>5}  {:>8}  {:>9}  {:>10}", "week", "active", "rels", "median_act");
    for &(week, count, rels, med) in &entity_snapshots {
        let flag = if count > n_nodes * 5 { "  !! explosion" } else { "" };
        println!("  {:>5}  {:>8}  {:>9}  {:>10.4}{}", week, count, rels, med, flag);
    }

    println!("\n── Lifecycle events (all time) ──");
    println!("  Born:            {}", lc.born);
    println!("  Split:           {}", lc.split);
    println!("  Merge:           {}", lc.merge);
    println!("  BecameDormant:   {}", lc.dormant);
    println!("  Revived:         {}", lc.revived);
    println!("  MembershipDelta: {}", lc.membership_delta);
    println!("  CoherenceShift:  {}", lc.coherence_shift);

    println!("\n── Partition quality ──");
    println!(
        "  Nodes in entities: {}  Active entities: {}  Ground-truth depts: {}",
        pred_labels.len(),
        n_active,
        n_depts_observed
    );
    println!("  NMI(engine | dept_labels): {:.4}", nmi_score);

    // Finding note for docs/eu-email-finding.md
    println!("\n── Finding summary ──");
    println!(
        "  Auto-threshold on real email distribution: {} active entities from {} nodes",
        n_active, n_nodes
    );
    println!(
        "  NMI = {:.4}  (1.0 = perfect match, 0.0 = random)",
        nmi_score
    );

    // Loose discovery assertion — tighten after baseline established
    assert!(lc.born >= 1, "no entities emerged from {} real email edges", edges.len());
}

/// Comparison run with a fixed `min_activity_threshold = 0.3`.
///
/// If auto-threshold causes entity explosion (threshold≈0 on smooth exponential
/// decay → all historical edges included → label propagation unstable) this test
/// should produce a much smaller, sane entity count (~42–200 active entities)
/// and higher NMI.
///
/// Run with:
///   cargo test -p graph-engine --test eu_email -- --ignored --nocapture eu_email_fixed_threshold_comparison
#[test]
#[ignore = "requires external data: data/email-Eu-core-temporal.txt"]
fn eu_email_fixed_threshold_comparison() {
    const FIXED_THRESHOLD: f32 = 0.3;

    let dir = data_dir();
    let edges = load_edges(&dir.join("email-Eu-core-temporal.txt"));
    let labels = load_labels(&dir.join("email-Eu-core-dept-labels.txt"));

    let mut nodes: BTreeSet<u64> = BTreeSet::new();
    for &(u, v, _) in &edges {
        nodes.insert(u);
        nodes.insert(v);
    }
    let n_nodes = nodes.len();

    let buckets = bucket_weekly(&edges);
    let total_weeks = buckets.len();

    let mut world = World::new();
    for &node in &nodes {
        world.insert_locus(Locus::new(LocusId(node), PERSON_KIND, StateVector::zeros(1)));
    }

    let cfg = InfluenceKindConfig::new("email")
        .with_decay(DECAY)
        .with_symmetric(true);
    let mut inf_reg = InfluenceKindRegistry::new();
    inf_reg.insert(EMAIL, cfg);

    let mut loci_reg = LocusKindRegistry::new();
    loci_reg.insert(PERSON_KIND, Box::new(EmailProgram));

    let engine = Engine::new(EngineConfig { max_batches_per_tick: 3 });
    let perspective = DefaultEmergencePerspective::default()
        .with_min_activity_threshold(FIXED_THRESHOLD);

    println!("\n═══ EU Email Fixed Threshold (min_activity={FIXED_THRESHOLD}) ═══");
    println!("  nodes: {}  threshold: {}", n_nodes, FIXED_THRESHOLD);

    for (week, pairs) in buckets.iter().enumerate() {
        if pairs.is_empty() {
            continue;
        }
        let stimuli = batch_stimuli(pairs);
        engine.tick(&mut world, &loci_reg, &inf_reg, stimuli);

        let is_checkpoint = (week + 1) % RECOGNIZE_EVERY == 0 || week + 1 == total_weeks;
        if is_checkpoint {
            engine.recognize_entities(&mut world, &inf_reg, &perspective);
        }
    }

    let active_entities: Vec<_> = world
        .entities()
        .iter()
        .filter(|e| e.status == EntityStatus::Active)
        .collect();
    let n_active = active_entities.len();

    let mut node_to_cluster: HashMap<u64, usize> = HashMap::new();
    for (ci, ent) in active_entities.iter().enumerate() {
        for &locus_id in &ent.current.members {
            node_to_cluster.insert(locus_id.0, ci);
        }
    }

    let mut pred_labels: Vec<usize> = Vec::new();
    let mut truth_labels: Vec<usize> = Vec::new();
    let mut dept_index: HashMap<u64, usize> = HashMap::new();

    for (&node, &dept) in &labels {
        if let Some(&cluster) = node_to_cluster.get(&node) {
            pred_labels.push(cluster);
            let next = dept_index.len();
            let idx = *dept_index.entry(dept).or_insert(next);
            truth_labels.push(idx);
        }
    }
    let n_depts_observed = dept_index.len();

    let nmi_score = if pred_labels.is_empty() {
        0.0
    } else {
        nmi_from_assignments(&pred_labels, &truth_labels, n_active, n_depts_observed)
    };

    let lc = count_lifecycle(&world);

    println!("\n── Fixed-threshold results ──");
    println!("  Active entities: {}  (nodes: {}  ratio: {:.1}×)",
             n_active, n_nodes, n_active as f64 / n_nodes as f64);
    println!("  Nodes in entities: {}  Ground-truth depts: {}",
             pred_labels.len(), n_depts_observed);
    println!("  NMI(engine | dept_labels): {:.4}", nmi_score);
    println!("  Born: {}  Dormant: {}  Revived: {}", lc.born, lc.dormant, lc.revived);

    // Diagnosis: if fixed threshold still explodes, auto-threshold is NOT the cause.
    // The likely culprit is activity decay collapsing the graph to isolated vertices.
    let explosion = n_active > n_nodes * 5;
    if explosion {
        println!("  DIAGNOSIS: explosion persists with fixed threshold={}.", FIXED_THRESHOLD);
        println!("             Auto-threshold is NOT the root cause.");
        println!("             Hypothesis: DECAY=0.5 (half-life 1 week) collapses all relationships");
        println!("             to ~0 by week 50 → no edges survive threshold → 986 singleton");
        println!("             communities per recognize call → cumulative Born explosion.");
        println!("             Next: run eu_email_slow_decay (DECAY=0.9) to confirm.");
    } else {
        println!("  DIAGNOSIS: fixed threshold → sane entity count. Auto-threshold root cause confirmed.");
    }

    assert!(lc.born >= 1, "no entities emerged");
}

/// DECAY sensitivity test: DECAY=0.9 (half-life ≈7 weeks) with auto-threshold.
///
/// DECAY=0.5 (half-life 1 week) collapses all relationship activity to ~0 by
/// week 50 on sporadic email data. This test uses 0.9 so that relationships
/// accumulate across infrequent contacts, matching the EU email cadence.
///
/// Expected: sane entity count (not >> n_nodes), higher NMI than DECAY=0.5.
/// If sane: Finding is "DECAY must be matched to data cadence; DECAY=0.5
/// is appropriate only for high-frequency synthetic workloads."
///
/// Run with:
///   cargo test -p graph-engine --test eu_email -- --ignored --nocapture eu_email_slow_decay
#[test]
#[ignore = "requires external data: data/email-Eu-core-temporal.txt"]
fn eu_email_slow_decay() {
    const SLOW_DECAY: f32 = 0.9; // half-life ≈7 weeks

    let dir = data_dir();
    let edges = load_edges(&dir.join("email-Eu-core-temporal.txt"));
    let labels = load_labels(&dir.join("email-Eu-core-dept-labels.txt"));

    let mut nodes: BTreeSet<u64> = BTreeSet::new();
    for &(u, v, _) in &edges {
        nodes.insert(u);
        nodes.insert(v);
    }
    let n_nodes = nodes.len();

    let buckets = bucket_weekly(&edges);
    let total_weeks = buckets.len();

    let mut world = World::new();
    for &node in &nodes {
        world.insert_locus(Locus::new(LocusId(node), PERSON_KIND, StateVector::zeros(1)));
    }

    let cfg = InfluenceKindConfig::new("email")
        .with_decay(SLOW_DECAY)
        .with_symmetric(true);
    let mut inf_reg = InfluenceKindRegistry::new();
    inf_reg.insert(EMAIL, cfg);

    let mut loci_reg = LocusKindRegistry::new();
    loci_reg.insert(PERSON_KIND, Box::new(EmailProgram));

    let engine = Engine::new(EngineConfig { max_batches_per_tick: 3 });
    let perspective = DefaultEmergencePerspective::default();

    println!("\n═══ EU Email Slow Decay (DECAY={SLOW_DECAY}, auto-threshold) ═══");
    println!("  nodes: {}  decay: {}", n_nodes, SLOW_DECAY);

    // (week, active, rels, median_act)
    let mut snapshots: Vec<(usize, usize, usize, f32)> = Vec::new();

    for (week, pairs) in buckets.iter().enumerate() {
        if pairs.is_empty() {
            continue;
        }
        let stimuli = batch_stimuli(pairs);
        engine.tick(&mut world, &loci_reg, &inf_reg, stimuli);

        let is_checkpoint = (week + 1) % RECOGNIZE_EVERY == 0 || week + 1 == total_weeks;
        if is_checkpoint {
            engine.recognize_entities(&mut world, &inf_reg, &perspective);
            let active = world
                .entities()
                .iter()
                .filter(|e| e.status == EntityStatus::Active)
                .count();
            let rel_count = world.relationships().iter().count();
            let mut acts: Vec<f32> = world
                .relationships()
                .iter()
                .map(|r| r.state.as_slice()[0].abs())
                .filter(|&a| a > 0.0)
                .collect();
            acts.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let median_act = acts.get(acts.len() / 2).copied().unwrap_or(0.0);
            snapshots.push((week + 1, active, rel_count, median_act));
        }
    }

    let active_entities: Vec<_> = world
        .entities()
        .iter()
        .filter(|e| e.status == EntityStatus::Active)
        .collect();
    let n_active = active_entities.len();

    let mut node_to_cluster: HashMap<u64, usize> = HashMap::new();
    for (ci, ent) in active_entities.iter().enumerate() {
        for &locus_id in &ent.current.members {
            node_to_cluster.insert(locus_id.0, ci);
        }
    }

    let mut pred_labels: Vec<usize> = Vec::new();
    let mut truth_labels: Vec<usize> = Vec::new();
    let mut dept_index: HashMap<u64, usize> = HashMap::new();
    for (&node, &dept) in &labels {
        if let Some(&cluster) = node_to_cluster.get(&node) {
            pred_labels.push(cluster);
            let next = dept_index.len();
            let idx = *dept_index.entry(dept).or_insert(next);
            truth_labels.push(idx);
        }
    }
    let n_depts_observed = dept_index.len();

    let nmi_score = if pred_labels.is_empty() {
        0.0
    } else {
        nmi_from_assignments(&pred_labels, &truth_labels, n_active, n_depts_observed)
    };

    let lc = count_lifecycle(&world);

    println!("\n── Slow-decay entity checkpoints ──");
    println!("  {:>5}  {:>8}  {:>9}  {:>10}", "week", "active", "rels", "median_act");
    for &(week, count, rels, med) in &snapshots {
        let flag = if count > n_nodes * 5 { "  !! explosion" } else { "" };
        println!("  {:>5}  {:>8}  {:>9}  {:>10.4}{}", week, count, rels, med, flag);
    }

    println!("\n── Slow-decay final results ──");
    println!("  Active entities: {}  (nodes: {}  ratio: {:.1}×)",
             n_active, n_nodes, n_active as f64 / n_nodes as f64);
    println!("  Nodes in entities: {}  Ground-truth depts: {}",
             pred_labels.len(), n_depts_observed);
    println!("  NMI(engine | dept_labels): {:.4}", nmi_score);
    println!("  Born: {}  Split: {}  Merge: {}  Dormant: {}  Revived: {}  MemberDelta: {}  CoherShift: {}",
             lc.born, lc.split, lc.merge, lc.dormant, lc.revived, lc.membership_delta, lc.coherence_shift);
    println!("  Born/Dormant ratio: {:.1}×  Born/Split ratio: {:.1}×",
             if lc.dormant > 0 { lc.born as f64 / lc.dormant as f64 } else { f64::INFINITY },
             if lc.split > 0 { lc.born as f64 / lc.split as f64 } else { f64::INFINITY });

    let explosion = n_active > n_nodes * 5;
    if !explosion {
        println!("\n  FINDING: DECAY=0.9 yields sane entity count ({} ≤ 5×{}).", n_active, n_nodes);
        println!("           DECAY must match data temporal cadence.");
        println!("           NMI={:.4}  (compare: DECAY=0.5 → 0.1002)", nmi_score);
    } else {
        println!("\n  WARNING: explosion persists even at DECAY=0.9. Further investigation needed.");
    }

    assert!(lc.born >= 1, "no entities emerged");
}
