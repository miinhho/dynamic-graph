use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use graph_core::{
    BatchId, Change, ChangeSubject, Endpoints, InfluenceKindId, Locus, LocusContext, LocusId,
    LocusKindId, LocusProgram, ProposedChange, Relationship, RelationshipLineage, StateVector, props,
};
use graph_engine::{
    DefaultEmergencePerspective, Engine, EngineConfig, InfluenceKindConfig, InfluenceKindRegistry,
    LocusKindRegistry, RelationshipSlotDef, Simulation, SimulationBuilder,
};
use graph_query::{connected_components, path_between, reachable_from};
use graph_testkit::fixtures::{chain_world, fan_in_world, ring_world, star_world, stimulus};
use graph_testkit::programs::InertProgram;
use graph_world::World;

fn bench_chain(c: &mut Criterion) {
    // chain_world(64, 0.9): signal propagates 64 hops, attenuates per batch
    c.bench_function("tick_chain_64", |b| {
        b.iter_batched(
            || chain_world(64, 0.9),
            |(mut world, loci, influences)| {
                let engine = Engine::new(EngineConfig::default());
                engine.tick(&mut world, &loci, &influences, vec![stimulus(1.0)])
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_star(c: &mut Criterion) {
    // star_world(32, 0.9): hub broadcasts to 32 arms
    c.bench_function("tick_star_32", |b| {
        b.iter_batched(
            || star_world(32, 0.9),
            |(mut world, loci, influences)| {
                let engine = Engine::new(EngineConfig::default());
                engine.tick(&mut world, &loci, &influences, vec![stimulus(1.0)])
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_ring(c: &mut Criterion) {
    // ring_world(16, 0.9): signal circulates and attenuates
    c.bench_function("tick_ring_16", |b| {
        b.iter_batched(
            || ring_world(16, 0.9),
            |(mut world, loci, influences)| {
                let engine = Engine::new(EngineConfig::default());
                engine.tick(&mut world, &loci, &influences, vec![stimulus(1.0)])
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_star_large(c: &mut Criterion) {
    c.bench_function("tick_star_512", |b| {
        b.iter_batched(
            || star_world(512, 0.9),
            |(mut world, loci, influences)| {
                let engine = Engine::new(EngineConfig::default());
                engine.tick(&mut world, &loci, &influences, vec![stimulus(1.0)])
            },
            BatchSize::LargeInput,
        );
    });
}

fn bench_fan_in(c: &mut Criterion) {
    c.bench_function("tick_fan_in_16x128_d32", |b| {
        b.iter_batched(
            || fan_in_world(16, 128, 32, 0.9),
            |(mut world, loci, influences)| {
                let engine = Engine::new(EngineConfig::default());
                engine.tick(&mut world, &loci, &influences, vec![stimulus(1.0)])
            },
            BatchSize::LargeInput,
        );
    });

    let mut group = c.benchmark_group("tick_fan_in_64x512_d128");
    group.sample_size(50);
    group.bench_function("default", |b| {
        b.iter_batched(
            || fan_in_world(64, 512, 128, 0.9),
            |(mut world, loci, influences)| {
                let engine = Engine::new(EngineConfig::default());
                engine.tick(&mut world, &loci, &influences, vec![stimulus(1.0)])
            },
            BatchSize::LargeInput,
        );
    });
    group.finish();
}

fn bench_fan_in_large(c: &mut Criterion) {
    let mut group = c.benchmark_group("tick_fan_in_256x4096_d128");
    group.sample_size(20);
    group.bench_function("default", |b| {
        b.iter_batched(
            || fan_in_world(256, 4096, 128, 0.9),
            |(mut world, loci, influences)| {
                let engine = Engine::new(EngineConfig::default());
                engine.tick(&mut world, &loci, &influences, vec![stimulus(1.0)])
            },
            BatchSize::LargeInput,
        );
    });
    group.finish();
}

// ── multi-step simulation ─────────────────────────────────────────────────

/// Steady-state `Simulation::step()` cost after relationships have emerged.
/// Each iteration runs one additional step on a warm world.
fn bench_simulation_step_steady(c: &mut Criterion) {
    let mut group = c.benchmark_group("simulation_step_steady");

    for (label, n) in [("chain_32", 32u64), ("chain_256", 256), ("ring_64", 64)] {
        group.bench_function(label, |b| {
            let (world, loci, influences) = if label.starts_with("ring") {
                ring_world(n, 0.9)
            } else {
                chain_world(n, 0.9)
            };
            let mut sim = Simulation::new(world, loci, influences);
            // Warm up: let relationships emerge and decay settle.
            sim.step(vec![stimulus(1.0)]);
            for _ in 0..19 {
                sim.step(vec![]);
            }
            b.iter(|| sim.step(vec![]));
        });
    }
    group.finish();
}

// ── causal ancestry walk ─────────────────────────────────────────────────

/// Cost of `causal_ancestors(last_change)` as DAG depth grows.
/// chain_world(N, 1.0): one tick produces a linear chain of N changes.
fn bench_causal_ancestors(c: &mut Criterion) {
    let mut group = c.benchmark_group("causal_ancestors_depth");

    for n in [16u64, 64, 256] {
        let label = format!("depth_{n}");
        group.bench_function(&label, |b| {
            b.iter_batched(
                || {
                    // gain=1.0 so signal never drops below threshold regardless of depth.
                    let (mut world, loci, influences) = chain_world(n, 1.0);
                    let engine = Engine::new(EngineConfig::default());
                    engine.tick(&mut world, &loci, &influences, vec![stimulus(1.0)]);
                    world
                },
                |world| {
                    let last_id = world.log().iter().last().unwrap().id;
                    world.log().causal_ancestors(last_id).len()
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

// ── changelog queries ─────────────────────────────────────────────────────

/// O(1) `get` vs O(k) `batch` vs O(k) `changes_to_locus` on a warm log.
fn bench_changelog_queries(c: &mut Criterion) {
    let mut group = c.benchmark_group("changelog_queries");

    // Build a world with a dense change log: 256-locus chain, 20 ticks.
    let (mut world, loci, influences) = chain_world(256, 0.9);
    let engine = Engine::new(EngineConfig::default());
    engine.tick(&mut world, &loci, &influences, vec![stimulus(1.0)]);
    for _ in 0..19 {
        engine.tick(&mut world, &loci, &influences, vec![]);
    }

    let mid_id = {
        let changes: Vec<_> = world.log().iter().collect();
        changes[changes.len() / 2].id
    };
    let mid_batch = world.log().get(mid_id).unwrap().batch;
    use graph_core::LocusId;
    let mid_locus = LocusId(128);

    group.bench_function("get_by_id", |b| {
        b.iter(|| world.log().get(mid_id))
    });
    group.bench_function("batch_scan", |b| {
        b.iter(|| world.log().batch(mid_batch).count())
    });
    group.bench_function("changes_to_locus", |b| {
        b.iter(|| world.log().changes_to_locus(mid_locus).count())
    });

    group.finish();
}

// ── relationship decay flush ──────────────────────────────────────────────────

/// Per-tick decay cost as relationship count grows.
///
/// Setup: stimulate a ring so all N relationships emerge; measure the first
/// quiescent step (no stimulus). That step's dominant work is decaying every
/// active relationship by its `decay_per_batch` multiplier.
fn bench_decay_flush(c: &mut Criterion) {
    let mut group = c.benchmark_group("decay_flush");

    for (label, n) in [("ring_16", 16u64), ("ring_64", 64), ("ring_256", 256)] {
        group.bench_function(label, |b| {
            b.iter_batched(
                || {
                    let (world, loci, influences) = ring_world(n, 0.9);
                    let mut sim = Simulation::new(world, loci, influences);
                    // All N relationships are active after one stimulus.
                    sim.step(vec![stimulus(1.0)]);
                    sim
                },
                // One quiescent step: decay pass runs over every relationship.
                |mut sim| sim.step(vec![]),
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

// ── BFS graph traversal ───────────────────────────────────────────────────────

/// `path_between`, `reachable_from`, and `connected_components` on warm rings.
///
/// Uses pre-warmed worlds (steady-state relationships emerged) so the bench
/// measures pure traversal cost, not emergence overhead.
fn bench_bfs_traversal(c: &mut Criterion) {
    let mut group = c.benchmark_group("bfs_traversal");

    let make_warm_ring = |n: u64| {
        let (world, loci, influences) = ring_world(n, 0.9);
        let mut sim = Simulation::new(world, loci, influences);
        sim.step(vec![stimulus(1.0)]);
        for _ in 0..9 {
            sim.step(vec![]);
        }
        sim.world
    };

    // path_between: O(V+E) BFS from L0 to the antipodal node.
    for n in [16u64, 64, 256] {
        let world = make_warm_ring(n);
        let far = LocusId(n / 2);
        group.bench_function(format!("path_between_ring_{n}"), |b| {
            b.iter(|| path_between(&world, LocusId(0), far))
        });
    }

    // reachable_from: full-depth BFS (depth ≥ n visits all nodes).
    for n in [16u64, 64, 256] {
        let world = make_warm_ring(n);
        let depth = n as usize;
        group.bench_function(format!("reachable_from_ring_{n}"), |b| {
            b.iter(|| reachable_from(&world, LocusId(0), depth))
        });
    }

    // connected_components: union-find-style BFS over all nodes.
    {
        let world = make_warm_ring(256);
        group.bench_function("connected_components_ring_256", |b| {
            b.iter(|| connected_components(&world))
        });
    }

    group.finish();
}

// ── entity emergence ──────────────────────────────────────────────────────

/// Entity recognition cost: label propagation + overlap matching + apply.
/// Measures the full recognize_entities pipeline on a warm world.
fn bench_emergence(c: &mut Criterion) {
    let mut group = c.benchmark_group("emergence");
    let perspective = DefaultEmergencePerspective::default();

    for (label, n) in [("ring_32", 32u64), ("ring_128", 128), ("ring_512", 512)] {
        group.bench_function(label, |b| {
            b.iter_batched(
                || {
                    let (world, loci, influences) = ring_world(n, 0.9);
                    let mut sim = Simulation::new(world, loci, influences);
                    // Warm: let relationships emerge.
                    sim.step(vec![stimulus(1.0)]);
                    for _ in 0..4 {
                        sim.step(vec![]);
                    }
                    sim
                },
                |mut sim| sim.recognize_entities(&perspective),
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

/// Emergence on a dense graph: star topology creates O(arms) relationships
/// from a single hub — tests component_stats on a single large component.
fn bench_emergence_dense(c: &mut Criterion) {
    let mut group = c.benchmark_group("emergence_dense");
    group.sample_size(50);
    let perspective = DefaultEmergencePerspective::default();

    for arms in [64u64, 256, 1024] {
        let label = format!("star_{arms}");
        group.bench_function(&label, |b| {
            b.iter_batched(
                || {
                    let (world, loci, influences) = star_world(arms, 0.9);
                    let mut sim = Simulation::new(world, loci, influences);
                    sim.step(vec![stimulus(1.0)]);
                    for _ in 0..4 {
                        sim.step(vec![]);
                    }
                    sim
                },
                |mut sim| sim.recognize_entities(&perspective),
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

// ── ingest pipeline ─────────────────────────────────────────────────────

/// Single ingest_named + flush: measures PropertyStore write, NameIndex
/// lookup, encode, and one engine tick.
fn bench_ingest_single(c: &mut Criterion) {
    c.bench_function("ingest_single_flush", |b| {
        b.iter_batched(
            || {
                SimulationBuilder::new()
                    .locus_kind("ENT", InertProgram)
                    .influence("signal", |cfg| cfg.with_decay(0.95))
                    .default_influence("signal")
                    .build()
            },
            |mut sim| {
                sim.ingest_named("node_0", "ENT", props! { "confidence" => 0.9_f64 });
                sim.flush_ingested()
            },
            BatchSize::SmallInput,
        );
    });
}

/// Batch ingest of N co-occurring entities + flush.
fn bench_ingest_batch(c: &mut Criterion) {
    let mut group = c.benchmark_group("ingest_batch");

    for n in [10usize, 50, 200] {
        let label = format!("cooccur_{n}");
        let names: Vec<String> = (0..n).map(|i| format!("entity_{i}")).collect();
        group.bench_function(&label, |b| {
            b.iter_batched(
                || {
                    let mut sim = SimulationBuilder::new()
                        .locus_kind("ENT", InertProgram)
                        .influence("signal", |cfg| cfg.with_decay(0.95))
                        .default_influence("signal")
                        .build();
                    for name in &names {
                        sim.ingest_named(
                            name,
                            "ENT",
                            props! { "confidence" => 0.5_f64 },
                        );
                    }
                    sim.flush_ingested();
                    sim
                },
                |mut sim| {
                    let entries: Vec<(&str, &str, graph_core::Properties)> = names
                        .iter()
                        .map(|name| {
                            (name.as_str(), "ENT", props! { "confidence" => 0.8_f64 })
                        })
                        .collect();
                    sim.ingest_batch_named(entries);
                    sim.flush_ingested()
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

/// Steady-state ingest: repeated ingest cycles on a warm simulation.
/// Measures incremental cost after relationships have already emerged.
fn bench_ingest_steady(c: &mut Criterion) {
    let mut group = c.benchmark_group("ingest_steady");

    for n in [20usize, 100] {
        let label = format!("batch_{n}");
        group.bench_function(&label, |b| {
            let mut sim = SimulationBuilder::new()
                .locus_kind("ENT", InertProgram)
                .influence("signal", |cfg| cfg.with_decay(0.95))
                .default_influence("signal")
                .build();
            // Warm up with multiple ingest cycles.
            for round in 0..5u64 {
                let warmup_names: Vec<String> = (0..n)
                    .map(|i| format!("ent_{}", round * n as u64 + i as u64))
                    .collect();
                let entries: Vec<(&str, &str, graph_core::Properties)> = warmup_names
                    .iter()
                    .map(|name| (name.as_str(), "ENT", props! { "confidence" => 0.7_f64 }))
                    .collect();
                sim.ingest_batch_named(entries);
                sim.flush_ingested();
            }

            let names: Vec<String> = (0..n).map(|i| format!("ent_{i}")).collect();
            b.iter(|| {
                let entries: Vec<(&str, &str, graph_core::Properties)> = names
                    .iter()
                    .map(|name| {
                        (name.as_str(), "ENT", props! { "confidence" => 0.9_f64 })
                    })
                    .collect();
                sim.ingest_batch_named(entries);
                sim.flush_ingested()
            });
        });
    }
    group.finish();
}

// ── NameIndex / PropertyStore lookup ─────────────────────────────────────

/// Name resolution throughput at scale.
fn bench_name_lookup(c: &mut Criterion) {
    let mut group = c.benchmark_group("name_lookup");

    for n in [100u64, 1000, 10000] {
        let label = format!("n_{n}");
        let names: Vec<String> = (0..n).map(|i| format!("entity_{i}")).collect();
        let mut sim = SimulationBuilder::new()
            .locus_kind("ENT", InertProgram)
            .influence("signal", |cfg| cfg.with_decay(0.95))
            .default_influence("signal")
            .build();
        for name in &names {
            sim.ingest_named(
                name,
                "ENT",
                props! { "confidence" => 0.5_f64 },
            );
        }
        sim.flush_ingested();

        // Lookup middle element.
        let target = format!("entity_{}", n / 2);
        group.bench_function(&label, |b| {
            b.iter(|| sim.resolve(&target))
        });
    }
    group.finish();
}

// ── subscriber + extra slots benchmarks ──────────────────────────────────────

/// Subscriber notification overhead as a function of subscriber count.
///
/// Setup: one "trigger" relationship. `n_subscribers` loci each subscribe to it.
/// Each tick: one ProposedChange on that relationship → engine delivers a
/// notification to all subscribers.
///
/// Measures: dispatch cost per relationship-change when subscriber count scales.
fn bench_subscriber_fanout(c: &mut Criterion) {
    // Simple sink program that just counts relationship notifications.
    struct RelSinkProgram;
    impl LocusProgram for RelSinkProgram {
        fn process(
            &self,
            _locus: &Locus,
            _incoming: &[&Change],
            _: &dyn LocusContext,
        ) -> Vec<ProposedChange> {
            Vec::new()
        }
    }

    // Locus that holds the monitored relationship's endpoints.
    struct RelHolderProgram;
    impl LocusProgram for RelHolderProgram {
        fn process(&self, _: &Locus, _: &[&Change], _: &dyn LocusContext) -> Vec<ProposedChange> {
            Vec::new()
        }
    }

    let mut group = c.benchmark_group("subscriber_fanout");

    for n_subs in [8u64, 64, 512, 4096] {
        let label = format!("subs_{n_subs}");
        group.bench_function(&label, |b| {
            b.iter_batched(
                || {
                    let conflict_kind = InfluenceKindId(1);
                    let holder_kind = LocusKindId(1);
                    let sub_kind = LocusKindId(2);

                    let mut world = World::new();
                    let mut loci_reg = LocusKindRegistry::new();
                    let mut inf_reg = InfluenceKindRegistry::new();

                    inf_reg.insert(
                        conflict_kind,
                        InfluenceKindConfig::new("conflict")
                            .with_extra_slots(vec![
                                RelationshipSlotDef::new("hostility", 0.0),
                                RelationshipSlotDef::new("engagement_count", 0.0),
                            ]),
                    );

                    // Two endpoint loci for the trigger relationship.
                    let ep_a = LocusId(0);
                    let ep_b = LocusId(1);
                    world.insert_locus(Locus::new(ep_a, holder_kind, StateVector::zeros(1)));
                    world.insert_locus(Locus::new(ep_b, holder_kind, StateVector::zeros(1)));
                    loci_reg.insert(holder_kind, Box::new(RelHolderProgram));

                    // Trigger relationship.
                    let rel_id = world.relationships_mut().mint_id();
                    world.relationships_mut().insert(Relationship {
                        id: rel_id,
                        kind: conflict_kind,
                        endpoints: Endpoints::Directed { from: ep_a, to: ep_b },
                        state: StateVector::from_slice(&[1.0, 0.0, 0.3, 0.0]),
                        lineage: RelationshipLineage {
                            created_by: None, last_touched_by: None,
                            change_count: 0, kinds_observed: vec![conflict_kind],
                        },
                        created_batch: BatchId(0),
                        last_decayed_batch: 0,
                        metadata: None,
                    });

                    // n_subs subscriber loci, all watching rel_id.
                    loci_reg.insert(sub_kind, Box::new(RelSinkProgram));
                    for i in 2..2 + n_subs {
                        let locus_id = LocusId(i);
                        world.insert_locus(Locus::new(locus_id, sub_kind, StateVector::zeros(1)));
                        world.subscriptions_mut().subscribe(locus_id, rel_id);
                    }

                    (world, loci_reg, inf_reg, rel_id, conflict_kind)
                },
                |(mut world, loci_reg, inf_reg, rel_id, conflict_kind)| {
                    let engine = Engine::new(EngineConfig::default());
                    engine.tick(
                        &mut world,
                        &loci_reg,
                        &inf_reg,
                        vec![ProposedChange::new(
                            ChangeSubject::Relationship(rel_id),
                            conflict_kind,
                            StateVector::from_slice(&[2.0, 0.0, 0.6, 1.0]),
                        )],
                    )
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

/// Cold-path cost: relationships with NO subscribers.
///
/// Baseline: shows that `has_subscribers()` O(1) check means zero-subscriber
/// relationships have negligible overhead vs. the original (no subscriptions at all).
fn bench_subscriber_cold_path(c: &mut Criterion) {
    let mut group = c.benchmark_group("subscriber_cold_path");

    // N relationships, each changed in one tick, zero subscribers.
    for n_rels in [64u64, 512, 4096] {
        let label = format!("rels_{n_rels}_no_subs");
        group.bench_function(&label, |b| {
            b.iter_batched(
                || {
                    let kind = InfluenceKindId(1);
                    let lk = LocusKindId(1);
                    let mut world = World::new();
                    let mut loci_reg = LocusKindRegistry::new();
                    let mut inf_reg = InfluenceKindRegistry::new();

                    inf_reg.insert(kind, InfluenceKindConfig::new("k"));
                    loci_reg.insert(lk, Box::new(InertProgram));

                    // Two endpoint loci.
                    world.insert_locus(Locus::new(LocusId(0), lk, StateVector::zeros(1)));
                    world.insert_locus(Locus::new(LocusId(1), lk, StateVector::zeros(1)));

                    // n_rels relationships, no subscribers.
                    let rel_ids: Vec<_> = (0..n_rels)
                        .map(|_| {
                            let id = world.relationships_mut().mint_id();
                            world.relationships_mut().insert(Relationship {
                                id,
                                kind,
                                endpoints: Endpoints::Directed {
                                    from: LocusId(0),
                                    to: LocusId(1),
                                },
                                state: StateVector::from_slice(&[1.0, 0.0]),
                                lineage: RelationshipLineage {
                                    created_by: None, last_touched_by: None,
                                    change_count: 0, kinds_observed: vec![kind],
                                },
                                created_batch: BatchId(0),
                                last_decayed_batch: 0,
                                metadata: None,
                            });
                            id
                        })
                        .collect();
                    (world, loci_reg, inf_reg, rel_ids, kind)
                },
                |(mut world, loci_reg, inf_reg, rel_ids, kind)| {
                    let engine = Engine::new(EngineConfig::default());
                    let stimuli = rel_ids
                        .iter()
                        .map(|&id| ProposedChange::new(
                            ChangeSubject::Relationship(id),
                            kind,
                            StateVector::from_slice(&[2.0, 0.0]),
                        ))
                        .collect();
                    engine.tick(&mut world, &loci_reg, &inf_reg, stimuli)
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

/// Extra-slot decay cost during flush: N relationships, each with K extra slots.
///
/// Shows how per-slot decay scales with slot count.
fn bench_extra_slot_decay_flush(c: &mut Criterion) {
    let mut group = c.benchmark_group("extra_slot_decay");

    for (label, n_rels, extra_slots) in [
        ("rels_1024_slots_0", 1024usize, 0usize),
        ("rels_1024_slots_2", 1024, 2),
        ("rels_1024_slots_8", 1024, 8),
    ] {
        group.bench_function(label, |b| {
            b.iter_batched(
                || {
                    let kind = InfluenceKindId(1);
                    let lk = LocusKindId(1);
                    let mut world = World::new();
                    let mut loci_reg = LocusKindRegistry::new();
                    let mut inf_reg = InfluenceKindRegistry::new();

                    let slots: Vec<RelationshipSlotDef> = (0..extra_slots)
                        .map(|i| {
                            RelationshipSlotDef::new(
                                // 'static str — use a fixed name; slot index is the distinguisher.
                                "metric",
                                1.0,
                            )
                            .with_decay(0.95 - i as f32 * 0.01)
                        })
                        .collect();

                    inf_reg.insert(
                        kind,
                        InfluenceKindConfig::new("k").with_decay(0.95).with_extra_slots(slots),
                    );
                    loci_reg.insert(lk, Box::new(InertProgram));

                    world.insert_locus(Locus::new(LocusId(0), lk, StateVector::zeros(1)));
                    world.insert_locus(Locus::new(LocusId(1), lk, StateVector::zeros(1)));

                    let mut initial = vec![1.0f32, 0.0];
                    initial.extend(std::iter::repeat_n(1.0, extra_slots));

                    for _ in 0..n_rels {
                        let id = world.relationships_mut().mint_id();
                        world.relationships_mut().insert(Relationship {
                            id,
                            kind,
                            endpoints: Endpoints::Directed {
                                from: LocusId(0),
                                to: LocusId(1),
                            },
                            state: StateVector::from_slice(&initial),
                            lineage: RelationshipLineage {
                                created_by: None, last_touched_by: None,
                                change_count: 0, kinds_observed: vec![kind],
                            },
                            created_batch: BatchId(0),
                            last_decayed_batch: 0,
                            metadata: None,
                        });
                    }

                    let engine = Engine::new(EngineConfig::default());
                    (world, inf_reg, engine)
                },
                |(mut world, inf_reg, engine)| {
                    // Advance batch to create a decay delta, then flush.
                    world.advance_batch();
                    world.advance_batch();
                    world.advance_batch();
                    engine.flush_relationship_decay(&mut world, &inf_reg)
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_chain,
    bench_star,
    bench_ring,
    bench_star_large,
    bench_fan_in,
    bench_fan_in_large,
    bench_simulation_step_steady,
    bench_causal_ancestors,
    bench_changelog_queries,
    bench_decay_flush,
    bench_bfs_traversal,
    bench_emergence,
    bench_emergence_dense,
    bench_ingest_single,
    bench_ingest_batch,
    bench_ingest_steady,
    bench_name_lookup,
    bench_subscriber_fanout,
    bench_subscriber_cold_path,
    bench_extra_slot_decay_flush,
);
criterion_main!(benches);
