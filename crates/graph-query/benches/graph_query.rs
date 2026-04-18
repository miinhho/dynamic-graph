use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use graph_core::{
    BatchId, Change, ChangeId, ChangeSubject, Endpoints, InfluenceKindId, KindObservation, Locus,
    LocusId, LocusKindId, Relationship, RelationshipLineage, StateVector,
};
use graph_query::api::{Query, RelSort, RelationshipPredicate, execute};
use graph_query::*;
use graph_world::World;
use smallvec::smallvec;
use std::hint::black_box;
use std::time::Duration;

// ─── Minimal LCG (no external deps) ──────────────────────────────────────────

struct Lcg(u64);
impl Lcg {
    fn new(seed: u64) -> Self {
        Self(seed)
    }
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0
    }
    fn range(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn f32_01(&mut self) -> f32 {
        (self.next() >> 11) as f32 / (1u64 << 53) as f32
    }
}

// ─── Graph builders ───────────────────────────────────────────────────────────

const LK: LocusKindId = LocusKindId(1);
const RK: InfluenceKindId = InfluenceKindId(1);

fn add_sym(world: &mut World, a: u64, b: u64, activity: f32) {
    let id = world.relationships_mut().mint_id();
    world.relationships_mut().insert(Relationship {
        id,
        kind: RK,
        endpoints: Endpoints::Symmetric {
            a: LocusId(a),
            b: LocusId(b),
        },
        state: StateVector::from_slice(&[activity, 0.5]),
        lineage: RelationshipLineage {
            created_by: None,
            last_touched_by: None,
            change_count: 1,
            kinds_observed: smallvec![KindObservation::synthetic(RK)],
        },
        created_batch: BatchId(0),
        last_decayed_batch: 0,
        metadata: None,
    });
}

fn add_dir(world: &mut World, from: u64, to: u64, activity: f32) {
    let id = world.relationships_mut().mint_id();
    world.relationships_mut().insert(Relationship {
        id,
        kind: RK,
        endpoints: Endpoints::Directed {
            from: LocusId(from),
            to: LocusId(to),
        },
        state: StateVector::from_slice(&[activity, 0.5]),
        lineage: RelationshipLineage {
            created_by: None,
            last_touched_by: None,
            change_count: 1,
            kinds_observed: smallvec![KindObservation::synthetic(RK)],
        },
        created_batch: BatchId(0),
        last_decayed_batch: 0,
        metadata: None,
    });
}

/// Erdős-Rényi style undirected graph: n nodes, avg_degree edges per node.
fn erdos_renyi(n: u64, avg_degree: f64, seed: u64) -> World {
    let mut rng = Lcg::new(seed);
    let mut w = World::new();
    for i in 0..n {
        w.insert_locus(Locus::new(LocusId(i), LK, StateVector::zeros(1)));
    }
    let p = avg_degree / (n - 1) as f64;
    for i in 0..n {
        for j in (i + 1)..n {
            if (rng.f32_01() as f64) < p {
                let act = rng.f32_01() * 0.8 + 0.2;
                add_sym(&mut w, i, j, act);
            }
        }
    }
    w
}

/// Directed random graph: each node i gets `m` directed edges to random j < i.
fn directed_random(n: u64, m: u64, seed: u64) -> World {
    let mut rng = Lcg::new(seed);
    let mut w = World::new();
    for i in 0..n {
        w.insert_locus(Locus::new(LocusId(i), LK, StateVector::zeros(1)));
    }
    for i in m..n {
        for _ in 0..m {
            let j = rng.range(i);
            let act = rng.f32_01() * 0.8 + 0.2;
            add_dir(&mut w, j, i, act);
        }
    }
    w
}

// ─── Pre-built worlds (shared across bench groups) ────────────────────────────

fn worlds() -> Vec<(&'static str, World)> {
    vec![
        ("n=200", erdos_renyi(200, 5.0, 42)),
        ("n=800", erdos_renyi(800, 5.0, 42)),
        ("n=3000", erdos_renyi(3000, 5.0, 42)),
    ]
}

fn dir_worlds() -> Vec<(&'static str, World)> {
    vec![
        ("n=200", directed_random(200, 3, 42)),
        ("n=800", directed_random(800, 3, 42)),
        ("n=3000", directed_random(3000, 3, 42)),
    ]
}

// ─── Traversal ────────────────────────────────────────────────────────────────

fn bench_traversal(c: &mut Criterion) {
    let mut g = c.benchmark_group("traversal");
    g.measurement_time(Duration::from_secs(5));

    for (label, world) in worlds() {
        let n = world.loci().len() as u64;

        g.bench_with_input(BenchmarkId::new("path_between", label), &world, |b, w| {
            b.iter(|| path_between(black_box(w), LocusId(0), LocusId(n - 1)))
        });

        g.bench_with_input(
            BenchmarkId::new("reachable_from/depth=3", label),
            &world,
            |b, w| b.iter(|| reachable_from(black_box(w), LocusId(0), 3)),
        );

        g.bench_with_input(
            BenchmarkId::new("connected_components", label),
            &world,
            |b, w| b.iter(|| connected_components(black_box(w))),
        );
    }

    for (label, world) in dir_worlds() {
        g.bench_with_input(BenchmarkId::new("has_cycle", label), &world, |b, w| {
            b.iter(|| has_cycle(black_box(w)))
        });
    }

    g.finish();
}

// ─── Centrality ───────────────────────────────────────────────────────────────

fn bench_centrality(c: &mut Criterion) {
    let mut g = c.benchmark_group("centrality");
    g.measurement_time(Duration::from_secs(8));

    // betweenness is O(V·E) — only small/medium
    for (label, world) in [
        ("n=200", erdos_renyi(200, 5.0, 42)),
        ("n=800", erdos_renyi(800, 5.0, 42)),
    ] {
        g.bench_with_input(
            BenchmarkId::new("all_betweenness", label),
            &world,
            |b, w| b.iter(|| all_betweenness(black_box(w))),
        );
    }

    // closeness is O(V·E) — small/medium/large
    for (label, world) in worlds() {
        g.bench_with_input(BenchmarkId::new("all_closeness", label), &world, |b, w| {
            b.iter(|| all_closeness(black_box(w)))
        });
    }

    // pagerank is O(iter·E) — all sizes
    for (label, world) in worlds() {
        g.bench_with_input(BenchmarkId::new("pagerank", label), &world, |b, w| {
            b.iter(|| pagerank(black_box(w), 0.85, 100, 1e-6))
        });
    }

    g.finish();
}

// ─── Community detection ──────────────────────────────────────────────────────

fn bench_community(c: &mut Criterion) {
    let mut g = c.benchmark_group("community");
    g.measurement_time(Duration::from_secs(8));

    for (label, world) in [
        ("n=200", erdos_renyi(200, 5.0, 42)),
        ("n=800", erdos_renyi(800, 5.0, 42)),
        ("n=3000", erdos_renyi(3000, 5.0, 42)),
    ] {
        g.bench_with_input(BenchmarkId::new("louvain", label), &world, |b, w| {
            b.iter(|| louvain(black_box(w)))
        });
    }

    g.finish();
}

// ─── Filter / query ───────────────────────────────────────────────────────────

fn bench_filter(c: &mut Criterion) {
    let mut g = c.benchmark_group("filter");
    g.measurement_time(Duration::from_secs(5));

    for (label, world) in worlds() {
        g.bench_with_input(
            BenchmarkId::new("relationships_with_activity", label),
            &world,
            |b, w| b.iter(|| relationships_with_activity(black_box(w), |a| a > 0.5)),
        );

        g.bench_with_input(BenchmarkId::new("hub_loci/min=5", label), &world, |b, w| {
            b.iter(|| hub_loci(black_box(w), 5))
        });
    }

    g.finish();
}

// ─── Optimizer: DirectLookup vs single-locus seed ────────────────────────────

/// Dense graph where every pair (from, to) has exactly one directed RK edge.
/// Used to benchmark seed selection: From(a) alone vs From(a)+To(b)+OfKind(k).
fn dense_directed(n: u64, seed: u64) -> World {
    let mut rng = Lcg::new(seed);
    let mut w = World::new();
    for i in 0..n {
        w.insert_locus(Locus::new(LocusId(i), LK, StateVector::zeros(1)));
    }
    // Build a complete directed graph (all i→j, i≠j).
    // This maximises the From(a) candidate set so the DirectLookup speedup is visible.
    for i in 0..n {
        for j in 0..n {
            if i != j {
                let act = rng.f32_01() * 0.6 + 0.2;
                add_dir(&mut w, i, j, act);
            }
        }
    }
    w
}

fn bench_direct_lookup(c: &mut Criterion) {
    let mut g = c.benchmark_group("optimizer/seed_selection");
    g.measurement_time(Duration::from_secs(5));

    // n=50: complete directed graph → 50×49 = 2 450 edges; From(0) seed = 49 candidates.
    // DirectLookup(From=0, To=49, Kind=RK) should resolve to exactly 1 edge in O(1).
    let world = dense_directed(50, 42);
    let n = world.loci().len() as u64;
    let target_to = LocusId(n - 1);
    let rk = InfluenceKindId(1);

    g.bench_function("from_only/n=50", |b| {
        let q = Query::FindRelationships {
            predicates: vec![RelationshipPredicate::From(LocusId(0))],
            sort_by: None,
            limit: None,
        };
        b.iter(|| execute(black_box(&world), black_box(&q)))
    });

    g.bench_function("direct_lookup(from+to+kind)/n=50", |b| {
        let q = Query::FindRelationships {
            predicates: vec![
                RelationshipPredicate::From(LocusId(0)),
                RelationshipPredicate::To(target_to),
                RelationshipPredicate::OfKind(rk),
            ],
            sort_by: None,
            limit: None,
        };
        b.iter(|| execute(black_box(&world), black_box(&q)))
    });

    g.bench_function("between(from+to)/n=50", |b| {
        let q = Query::FindRelationships {
            predicates: vec![
                RelationshipPredicate::From(LocusId(0)),
                RelationshipPredicate::To(target_to),
            ],
            sort_by: None,
            limit: None,
        };
        b.iter(|| execute(black_box(&world), black_box(&q)))
    });

    g.finish();
}

// ─── Optimizer: lazy limit vs full-collection ──────────────────────────────

fn bench_lazy_limit(c: &mut Criterion) {
    let mut g = c.benchmark_group("optimizer/lazy_limit");
    g.measurement_time(Duration::from_secs(5));

    // Large graph with many matching edges; limit=1 should short-circuit immediately.
    for (label, world) in worlds() {
        let n = world.loci().len() as u64;

        // No sort + limit: should use lazy .take(n) and terminate early.
        g.bench_with_input(
            BenchmarkId::new("no_sort+limit=1", label),
            &world,
            |b, w| {
                let q = Query::FindRelationships {
                    predicates: vec![RelationshipPredicate::ActivityAbove(0.0)],
                    sort_by: None,
                    limit: Some(1),
                };
                b.iter(|| execute(black_box(w), black_box(&q)))
            },
        );

        // With sort: must collect all, sort, then truncate — no early exit.
        g.bench_with_input(
            BenchmarkId::new("sort_desc+limit=1", label),
            &world,
            |b, w| {
                let q = Query::FindRelationships {
                    predicates: vec![RelationshipPredicate::ActivityAbove(0.0)],
                    sort_by: Some(RelSort::ActivityDesc),
                    limit: Some(1),
                };
                b.iter(|| execute(black_box(w), black_box(&q)))
            },
        );

        // Baseline: no limit, no sort.
        g.bench_with_input(
            BenchmarkId::new("no_sort+no_limit", label),
            &world,
            |b, w| {
                let q = Query::FindRelationships {
                    predicates: vec![RelationshipPredicate::ActivityAbove(0.0)],
                    sort_by: None,
                    limit: None,
                };
                b.iter(|| execute(black_box(w), black_box(&q)))
            },
        );

        let _ = n; // suppress unused warning
    }

    g.finish();
}

// ─── Optimizer: activity-aware traversal ──────────────────────────────────────

/// Graph with `active_ratio` fraction of edges above threshold.
/// The rest are dormant (activity=0.05, below threshold 0.3).
fn mixed_activity_graph(n: u64, avg_degree: f64, active_ratio: f64, seed: u64) -> World {
    let mut rng = Lcg::new(seed);
    let p = avg_degree / (n - 1) as f64;
    let mut w = World::new();
    for i in 0..n {
        w.insert_locus(Locus::new(LocusId(i), LK, StateVector::zeros(1)));
    }
    for i in 0..n {
        for j in (i + 1)..n {
            if (rng.f32_01() as f64) < p {
                // active_ratio fraction gets activity 0.7; rest get 0.05 (dormant)
                let active = (rng.next() % 1000) as f64 / 1000.0 < active_ratio;
                let act = if active { 0.7f32 } else { 0.05 };
                add_sym(&mut w, i, j, act);
            }
        }
    }
    w
}

fn bench_active_traversal(c: &mut Criterion) {
    let mut g = c.benchmark_group("optimizer/active_traversal");
    g.measurement_time(Duration::from_secs(6));

    // 50% dormant edges — reachable_from will traverse dead edges; active skips them.
    for (label, n) in [("n=200", 200u64), ("n=800", 800u64), ("n=3000", 3000u64)] {
        let world_mixed = mixed_activity_graph(n, 5.0, 0.5, 42);
        let world_sparse = mixed_activity_graph(n, 5.0, 0.2, 42); // 80% dormant

        g.bench_with_input(
            BenchmarkId::new("reachable_from/depth=3/mixed50%", label),
            &world_mixed,
            |b, w| b.iter(|| reachable_from(black_box(w), LocusId(0), 3)),
        );

        g.bench_with_input(
            BenchmarkId::new("reachable_from_active/depth=3/mixed50%", label),
            &world_mixed,
            |b, w| b.iter(|| reachable_from_active(black_box(w), LocusId(0), 3, 0.3)),
        );

        g.bench_with_input(
            BenchmarkId::new("reachable_from/depth=3/sparse20%", label),
            &world_sparse,
            |b, w| b.iter(|| reachable_from(black_box(w), LocusId(0), 3)),
        );

        g.bench_with_input(
            BenchmarkId::new("reachable_from_active/depth=3/sparse20%", label),
            &world_sparse,
            |b, w| b.iter(|| reachable_from_active(black_box(w), LocusId(0), 3, 0.3)),
        );
    }

    g.finish();
}

// ─── Phase 1 B4: path_between scaling ────────────────────────────────────────

/// path_between scaling over N ∈ [64, 256, 1024] on Erdős-Rényi graphs.
/// Uses BenchmarkId so results appear as path_scaling/64, path_scaling/256, etc.
fn bench_path_scaling(c: &mut Criterion) {
    let mut g = c.benchmark_group("path_scaling");
    g.measurement_time(Duration::from_secs(5));

    for n in [64u64, 256, 1024] {
        let world = erdos_renyi(n, 5.0, 42);
        let far = LocusId(n - 1);
        g.bench_with_input(BenchmarkId::new("path_between", n), &world, |b, w| {
            b.iter(|| path_between(black_box(w), LocusId(0), far))
        });
    }

    g.finish();
}

// ─── Phase 1 B4: reachable_from_active scaling ───────────────────────────────

/// reachable_from_active scaling over N ∈ [64, 256, 1024].
/// Uses mixed-activity graph (50% dormant) to exercise the active-traversal path.
/// Activity threshold 0.3 matches existing bench_active_traversal convention.
fn bench_reach_active_scaling(c: &mut Criterion) {
    let mut g = c.benchmark_group("reach_active_scaling");
    g.measurement_time(Duration::from_secs(5));

    for n in [64u64, 256, 1024] {
        let world = mixed_activity_graph(n, 5.0, 0.5, 42);
        g.bench_with_input(
            BenchmarkId::new("reachable_from_active", n),
            &world,
            |b, w| b.iter(|| reachable_from_active(black_box(w), LocusId(0), n as usize, 0.3)),
        );
    }

    g.finish();
}

// ─── Phase 1 B4: causal_ancestors scaling ────────────────────────────────────

/// Build a world whose ChangeLog contains a linear predecessor chain of exactly
/// `depth` changes: change[i] has change[i-1] as its sole predecessor.
/// Returns the world and the last ChangeId.
fn linear_change_chain(depth: usize) -> (World, ChangeId) {
    let kind = InfluenceKindId(1);
    let lk = LocusKindId(1);
    let mut world = World::new();
    world.insert_locus(Locus::new(LocusId(0), lk, StateVector::zeros(1)));

    let before = StateVector::zeros(1);
    let after = StateVector::from_slice(&[1.0]);

    let mut prev: Option<ChangeId> = None;
    let mut last_id = ChangeId(0);

    for i in 0..depth {
        let id = ChangeId(i as u64);
        let predecessors = prev.map(|p| vec![p]).unwrap_or_default();
        let change = Change {
            id,
            subject: ChangeSubject::Locus(LocusId(0)),
            kind,
            predecessors,
            before: before.clone(),
            after: after.clone(),
            batch: BatchId(i as u64),
            wall_time: None,
            metadata: None,
        };
        world.append_change(change);
        prev = Some(id);
        last_id = id;
    }

    (world, last_id)
}

/// causal_ancestors BFS cost as ChangeLog depth grows: ∈ [10, 100, 500].
fn bench_causal_scaling(c: &mut Criterion) {
    let mut g = c.benchmark_group("causal_scaling");
    g.measurement_time(Duration::from_secs(5));

    for depth in [10usize, 100, 500] {
        let (world, last_id) = linear_change_chain(depth);
        g.bench_with_input(
            BenchmarkId::new("causal_ancestors", depth),
            &(world, last_id),
            |b, (w, lid)| b.iter(|| causal_ancestors(black_box(w), *lid)),
        );
    }

    g.finish();
}

// ─── B4: counterfactual replay scaling ───────────────────────────────────────

/// Build a world with a linear causal chain of `depth` changes, returning the
/// world and the root ChangeId (id=0) and last ChangeId.
fn causal_chain_world(depth: usize) -> (World, ChangeId) {
    use graph_core::{Change, ChangeId, ChangeSubject, InfluenceKindId, LocusKindId};
    let kind = InfluenceKindId(1);
    let lk = LocusKindId(1);
    let mut world = World::new();
    world.insert_locus(Locus::new(LocusId(0), lk, StateVector::zeros(1)));

    let before = StateVector::zeros(1);
    let after = StateVector::from_slice(&[1.0]);
    let mut prev: Option<ChangeId> = None;

    for i in 0..depth {
        let id = ChangeId(i as u64);
        let predecessors = prev.map(|p| vec![p]).unwrap_or_default();
        world.append_change(Change {
            id,
            subject: ChangeSubject::Locus(LocusId(0)),
            kind,
            predecessors,
            before: before.clone(),
            after: after.clone(),
            batch: BatchId(i as u64),
            wall_time: None,
            metadata: None,
        });
        prev = Some(id);
    }

    (world, ChangeId(0))
}

/// counterfactual_replay cost as causal chain depth grows.
/// Removing the root suppresses the entire chain — O(chain depth) BFS + O(R) rels.
fn bench_counterfactual_scaling(c: &mut Criterion) {
    let mut g = c.benchmark_group("counterfactual_scaling");
    g.measurement_time(Duration::from_secs(5));

    for depth in [10usize, 100, 500] {
        let (world, root) = causal_chain_world(depth);
        g.bench_with_input(
            BenchmarkId::new("counterfactual_replay/chain", depth),
            &(world, root),
            |b, (w, r)| b.iter(|| graph_query::counterfactual_replay(black_box(w), vec![*r])),
        );
    }

    g.finish();
}

// ─── Entry point ─────────────────────────────────────────────────────────────

criterion_group!(
    benches,
    bench_traversal,
    bench_centrality,
    bench_community,
    bench_filter,
    bench_direct_lookup,
    bench_lazy_limit,
    bench_active_traversal,
    bench_path_scaling,
    bench_reach_active_scaling,
    bench_causal_scaling,
    bench_counterfactual_scaling,
);
criterion_main!(benches);
