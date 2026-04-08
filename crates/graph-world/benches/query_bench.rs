use criterion::{Criterion, black_box, criterion_group, criterion_main};
use graph_core::{EntityId, EntityKindId};
use graph_testkit::{dynamic_channel_world, representative_query_world};
use graph_world::{EntitySelector, SelectorMode, SnapshotQuery};

fn query_benchmarks(c: &mut Criterion) {
    let world = dynamic_channel_world();
    let medium_world = representative_query_world(128);
    let large_world = representative_query_world(512);
    let snapshot = world.snapshot();
    let medium_snapshot = medium_world.snapshot();
    let large_snapshot = large_world.snapshot();
    let query = SnapshotQuery::new(snapshot);
    let medium_query = SnapshotQuery::new(medium_snapshot);
    let large_query = SnapshotQuery::new(large_snapshot);
    let selector = EntitySelector {
        source: EntityId(1),
        targets: Vec::new(),
        target_kinds: vec![EntityKindId(1)],
        radius: Some(2.0),
        mode: SelectorMode::IndexedOnly,
    };
    let medium_selector = EntitySelector {
        source: EntityId(1),
        targets: Vec::new(),
        target_kinds: vec![EntityKindId(1)],
        radius: Some(4.0),
        mode: SelectorMode::IndexedOnly,
    };
    let large_selector = EntitySelector {
        source: EntityId(1),
        targets: Vec::new(),
        target_kinds: vec![EntityKindId(1)],
        radius: Some(6.0),
        mode: SelectorMode::IndexedOnly,
    };

    let mut group = c.benchmark_group("world_query");
    group.bench_function("entities_by_kind", |b| {
        b.iter(|| black_box(query.entities().kind(EntityKindId(1)).collect()))
    });
    group.bench_function("selector_select", |b| {
        b.iter(|| black_box(query.entities().select(&selector).ids().to_vec()))
    });
    group.bench_function("channels_to_target", |b| {
        b.iter(|| black_box(query.channels().to(EntityId(2)).ids().to_vec()))
    });
    group.bench_function("selector_select_128", |b| {
        b.iter(|| {
            black_box(
                medium_query
                    .entities()
                    .select(&medium_selector)
                    .ids()
                    .to_vec(),
            )
        })
    });
    group.bench_function("channels_to_target_128", |b| {
        b.iter(|| black_box(medium_query.channels().to(EntityId(32)).ids().to_vec()))
    });
    group.bench_function("selector_select_512", |b| {
        b.iter(|| {
            black_box(
                large_query
                    .entities()
                    .select(&large_selector)
                    .ids()
                    .to_vec(),
            )
        })
    });
    group.bench_function("channels_to_target_512", |b| {
        b.iter(|| black_box(large_query.channels().to(EntityId(128)).ids().to_vec()))
    });
    group.finish();
}

criterion_group!(benches, query_benchmarks);
criterion_main!(benches);
