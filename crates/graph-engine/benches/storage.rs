//! Storage benchmarks: `commit_batch` and `save_world` / `load_world`.
//!
//! Requires the `storage` feature:
//!
//! ```text
//! cargo bench --bench storage --features storage
//! ```

use criterion::{criterion_group, criterion_main, BatchSize, Criterion};

use graph_core::BatchId;
use graph_engine::Simulation;
use graph_storage::Storage;
use graph_testkit::fixtures::{ring_world, stimulus};

/// Per-batch incremental write cost as world size scales.
///
/// Setup: ring world stimulated once so relationships emerge. Measures one
/// `commit_batch` call on the batch just committed.
fn bench_commit_batch(c: &mut Criterion) {
    let mut group = c.benchmark_group("storage_commit_batch");
    group.sample_size(50);

    for (label, n) in [("ring_16", 16u64), ("ring_64", 64), ("ring_256", 256)] {
        group.bench_function(label, |b| {
            b.iter_batched(
                || {
                    let (world, loci, influences) = ring_world(n, 0.9);
                    let mut sim = Simulation::new(world, loci, influences);
                    sim.step(vec![stimulus(1.0)]);
                    let dir = tempfile::tempdir().unwrap();
                    let path = dir.path().join("bench.redb");
                    let storage = Storage::open(&path).unwrap();
                    // Seed the file with a snapshot so commit_batch extends it.
                    let world = sim.into_world();
                    storage.save_world(&world).unwrap();
                    // commit_batch expects a batch index that was committed.
                    let committed_batch = BatchId(world.current_batch().0.saturating_sub(1));
                    (storage, world, committed_batch, dir)
                },
                |(storage, world, batch, _dir)| {
                    storage.commit_batch(&world, batch).unwrap();
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

/// Full save + fresh load roundtrip cost as world size scales.
///
/// Each iteration opens a new database, writes a full snapshot with
/// `save_world`, then reads it back with `load_world`.
fn bench_save_load_world(c: &mut Criterion) {
    let mut group = c.benchmark_group("storage_save_load");
    group.sample_size(20);

    for (label, n) in [("ring_16", 16u64), ("ring_64", 64), ("ring_256", 256)] {
        group.bench_function(label, |b| {
            b.iter_batched(
                || {
                    let (world, loci, influences) = ring_world(n, 0.9);
                    let mut sim = Simulation::new(world, loci, influences);
                    sim.step(vec![stimulus(1.0)]);
                    for _ in 0..4 {
                        sim.step(vec![]);
                    }
                    let dir = tempfile::tempdir().unwrap();
                    let path = dir.path().join("bench.redb");
                    (sim.into_world(), path, dir)
                },
                |(world, path, _dir)| {
                    let storage = Storage::open(&path).unwrap();
                    storage.save_world(&world).unwrap();
                    storage.load_world().unwrap()
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

criterion_group!(storage_benches, bench_commit_batch, bench_save_load_world,);
criterion_main!(storage_benches);
