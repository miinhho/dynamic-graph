use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use graph_testkit::fixtures::{chain_world, fan_in_world, ring_world, star_world, stimulus};
use graph_engine::{Engine, EngineConfig};

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
    let mut group = c.benchmark_group("tick_star_512");
    // Sequential (threshold = usize::MAX → never parallel)
    group.bench_function("sequential", |b| {
        b.iter_batched(
            || star_world(512, 0.9),
            |(mut world, loci, influences)| {
                let engine = Engine::new(EngineConfig {
                    parallel_dispatch_min_loci: usize::MAX,
                    ..Default::default()
                });
                engine.tick(&mut world, &loci, &influences, vec![stimulus(1.0)])
            },
            BatchSize::LargeInput,
        );
    });
    // Parallel (threshold = 0 → always parallel)
    group.bench_function("parallel", |b| {
        b.iter_batched(
            || star_world(512, 0.9),
            |(mut world, loci, influences)| {
                let engine = Engine::new(EngineConfig {
                    parallel_dispatch_min_loci: 0,
                    ..Default::default()
                });
                engine.tick(&mut world, &loci, &influences, vec![stimulus(1.0)])
            },
            BatchSize::LargeInput,
        );
    });
    group.finish();
}

fn bench_fan_in(c: &mut Criterion) {
    // 16 sources × 128 sinks × 32 dims: O(n_sources × dims) float ops per sink locus
    let mut group = c.benchmark_group("tick_fan_in_16x128_d32");
    group.bench_function("sequential", |b| {
        b.iter_batched(
            || fan_in_world(16, 128, 32, 0.9),
            |(mut world, loci, influences)| {
                let engine = Engine::new(EngineConfig {
                    parallel_dispatch_min_loci: usize::MAX,
                    ..Default::default()
                });
                engine.tick(&mut world, &loci, &influences, vec![stimulus(1.0)])
            },
            BatchSize::LargeInput,
        );
    });
    group.bench_function("parallel", |b| {
        b.iter_batched(
            || fan_in_world(16, 128, 32, 0.9),
            |(mut world, loci, influences)| {
                let engine = Engine::new(EngineConfig {
                    parallel_dispatch_min_loci: 0,
                    ..Default::default()
                });
                engine.tick(&mut world, &loci, &influences, vec![stimulus(1.0)])
            },
            BatchSize::LargeInput,
        );
    });
    group.finish();

    // 64 sources × 512 sinks × 128 dims: ~8 K float ops per sink — heavier workload
    let mut group = c.benchmark_group("tick_fan_in_64x512_d128");
    group.sample_size(50);
    group.bench_function("sequential", |b| {
        b.iter_batched(
            || fan_in_world(64, 512, 128, 0.9),
            |(mut world, loci, influences)| {
                let engine = Engine::new(EngineConfig {
                    parallel_dispatch_min_loci: usize::MAX,
                    ..Default::default()
                });
                engine.tick(&mut world, &loci, &influences, vec![stimulus(1.0)])
            },
            BatchSize::LargeInput,
        );
    });
    group.bench_function("parallel", |b| {
        b.iter_batched(
            || fan_in_world(64, 512, 128, 0.9),
            |(mut world, loci, influences)| {
                let engine = Engine::new(EngineConfig {
                    parallel_dispatch_min_loci: 0,
                    ..Default::default()
                });
                engine.tick(&mut world, &loci, &influences, vec![stimulus(1.0)])
            },
            BatchSize::LargeInput,
        );
    });
    group.finish();
}

fn bench_fan_in_large(c: &mut Criterion) {
    // 수천 loci 규모: 실제 프로덕션 사용 규모 시뮬레이션
    // 256 sources × 4096 sinks × 128 dims
    //   → sink 당 256×128 = 32K float ops, 총 ~130M ops
    let mut group = c.benchmark_group("tick_fan_in_256x4096_d128");
    group.sample_size(20);
    group.bench_function("sequential", |b| {
        b.iter_batched(
            || fan_in_world(256, 4096, 128, 0.9),
            |(mut world, loci, influences)| {
                let engine = Engine::new(EngineConfig {
                    parallel_dispatch_min_loci: usize::MAX,
                    ..Default::default()
                });
                engine.tick(&mut world, &loci, &influences, vec![stimulus(1.0)])
            },
            BatchSize::LargeInput,
        );
    });
    group.bench_function("parallel", |b| {
        b.iter_batched(
            || fan_in_world(256, 4096, 128, 0.9),
            |(mut world, loci, influences)| {
                let engine = Engine::new(EngineConfig {
                    parallel_dispatch_min_loci: 64,
                    ..Default::default()
                });
                engine.tick(&mut world, &loci, &influences, vec![stimulus(1.0)])
            },
            BatchSize::LargeInput,
        );
    });
    group.finish();
}

criterion_group!(benches, bench_chain, bench_star, bench_ring, bench_star_large, bench_fan_in, bench_fan_in_large);
criterion_main!(benches);
