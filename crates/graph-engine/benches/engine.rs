use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use graph_testkit::fixtures::{chain_world, ring_world, star_world, stimulus};
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

criterion_group!(benches, bench_chain, bench_star, bench_ring);
criterion_main!(benches);
