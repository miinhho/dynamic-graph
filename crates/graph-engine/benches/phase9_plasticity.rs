use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use graph_core::{BatchId, InfluenceKindId};
use graph_engine::{PairPredictionObjective, PlasticityConfig};
use graph_testkit::sociopatterns::{
    RankSignal, SocioPatternsProfile, evaluate_next_block_prediction, run_stream,
};

const SEED: u64 = 0x50c10_5ca77e5d;
const TOP_KS: [usize; 3] = [20, 50, 100];

fn bench_phase9_stream_scaling(c: &mut Criterion) {
    let profiles = [
        (SocioPatternsProfile::medium(), 90usize),
        (SocioPatternsProfile::school_scale(), 120usize),
        (SocioPatternsProfile::xlarge(), 160usize),
    ];

    let mut group = c.benchmark_group("phase9_stream_scaling");
    group.sample_size(10);

    for (profile, train_blocks) in profiles {
        let events = (profile.events_per_block * train_blocks) as u64;
        group.throughput(Throughput::Elements(events));
        group.bench_with_input(
            BenchmarkId::from_parameter(profile.name),
            &profile,
            |b, profile| {
                b.iter_batched(
                    || (),
                    |_| {
                        run_stream(
                            *profile,
                            train_blocks,
                            SEED,
                            PlasticityConfig {
                                learning_rate: 0.05,
                                weight_decay: 0.995,
                                max_weight: 5.0,
                            },
                        )
                    },
                    BatchSize::LargeInput,
                );
            },
        );
    }

    group.finish();
}

fn bench_phase9_prediction_pipeline(c: &mut Criterion) {
    let cases = [
        (SocioPatternsProfile::medium(), 90usize, 30usize),
        (SocioPatternsProfile::school_scale(), 120usize, 60usize),
        (SocioPatternsProfile::xlarge(), 160usize, 80usize),
    ];

    let mut group = c.benchmark_group("phase9_prediction_pipeline");
    group.sample_size(10);

    for (profile, train_blocks, test_blocks) in cases {
        let windows = (train_blocks + test_blocks) as u64;
        group.throughput(Throughput::Elements(windows));

        for rank_signal in [RankSignal::Activity, RankSignal::Strength] {
            let id = BenchmarkId::new(profile.name, rank_signal.label());
            group.bench_function(id, |b| {
                b.iter(|| {
                    evaluate_next_block_prediction(
                        profile,
                        SEED,
                        train_blocks,
                        test_blocks,
                        PlasticityConfig {
                            learning_rate: 0.05,
                            weight_decay: 0.995,
                            max_weight: 5.0,
                        },
                        rank_signal,
                        &TOP_KS,
                    )
                });
            });
        }
    }

    group.finish();
}

fn bench_phase9_objective_window(c: &mut Criterion) {
    let cases = [
        (SocioPatternsProfile::medium(), 90usize, 30usize),
        (SocioPatternsProfile::school_scale(), 120usize, 60usize),
    ];

    let mut group = c.benchmark_group("phase9_objective_window");
    group.sample_size(10);

    for (profile, train_blocks, test_blocks) in cases {
        let plasticity = PlasticityConfig {
            learning_rate: 0.05,
            weight_decay: 0.995,
            max_weight: 5.0,
        };
        let train_run = run_stream(profile, train_blocks, SEED, plasticity);
        let full_run = run_stream(profile, train_blocks + test_blocks, SEED, plasticity);
        let objective = PairPredictionObjective {
            kind: InfluenceKindId(300),
            k: 50,
            horizon_batches: test_blocks as u64,
            recall_weight: 0.5,
        };
        let from_batch = BatchId(train_blocks as u64);
        let to_batch = BatchId((train_blocks + test_blocks - 1) as u64);

        group.bench_function(BenchmarkId::new(profile.name, "rank"), |b| {
            b.iter(|| objective.rank(&train_run.world))
        });
        group.bench_function(BenchmarkId::new(profile.name, "score_window"), |b| {
            b.iter(|| {
                objective.score_window(&train_run.world, &full_run.event_log, from_batch, to_batch)
            })
        });
    }

    group.finish();
}

criterion_group!(
    phase9,
    bench_phase9_stream_scaling,
    bench_phase9_prediction_pipeline,
    bench_phase9_objective_window
);
criterion_main!(phase9);
