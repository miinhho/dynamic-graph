use criterion::{Criterion, black_box, criterion_group, criterion_main};
use graph_core::{
    Channel, Emission, EmissionLaw, EntityKindId, EntityProgram, EntityState, InteractionKind,
    LawId, SignalVector, StateVector, Stimulus,
};
use graph_engine::{
    __bench::{
        aggregate_cohort_emissions, compute_state_update_present, dispatch_source_emission_count,
        merge_stimuli, plan_channel_dispatch_target_count,
    },
    BasicStabilizer, EngineConfig, LawRegistry, ProgramRegistry, RoutingPolicy,
};
use graph_testkit::{
    dynamic_channel_world, emitting_entity, entity, field_channel, pairwise_channel,
    world_with_components,
};

struct LinearProgram;
struct ExcitatoryLaw;

impl EntityProgram for LinearProgram {
    fn next_state(
        &self,
        current: &EntityState,
        inbox: &SignalVector,
        stimulus: Option<&Stimulus>,
    ) -> EntityState {
        let external_signal = stimulus
            .map(|input| input.signal.clone())
            .unwrap_or_default();
        let combined = inbox.add(&external_signal);
        EntityState {
            internal: current
                .internal
                .add(&StateVector::from(combined.scaled(0.5))),
            emitted: combined,
            cooldown: 0,
        }
    }
}

impl EmissionLaw for ExcitatoryLaw {
    fn project(&self, outbound: &SignalVector, _: &EntityState, channel: &Channel) -> SignalVector {
        outbound.scaled(channel.weight * (1.0_f32 - channel.attenuation).clamp(0.0, 1.0))
    }

    fn kind(&self) -> InteractionKind {
        InteractionKind::Excitatory
    }
}

fn registries() -> (ProgramRegistry, LawRegistry) {
    let mut programs = ProgramRegistry::default();
    programs.insert(EntityKindId(1), Box::new(LinearProgram));
    programs.insert(EntityKindId(2), Box::new(LinearProgram));
    programs.insert(EntityKindId(3), Box::new(LinearProgram));

    let mut laws = LawRegistry::default();
    laws.insert(LawId(1), Box::new(ExcitatoryLaw));

    (programs, laws)
}

fn internal_runtime_benchmarks(c: &mut Criterion) {
    let world = dynamic_channel_world();
    let snapshot = world.snapshot();
    let pairwise_only_world = world_with_components(
        [emitting_entity(1, 1, 0.0, 1.0), entity(2, 1, 1.0)],
        [pairwise_channel(1, 1, 2, 1)],
    );
    let pairwise_only_snapshot = pairwise_only_world.snapshot();
    let field_only_world = world_with_components(
        [
            emitting_entity(1, 1, 0.0, 1.0),
            entity(2, 1, 1.0),
            entity(3, 2, 3.0),
        ],
        [field_channel(3, 1, 2.0, 1)],
    );
    let field_only_snapshot = field_only_world.snapshot();
    let (programs, laws) = registries();
    let stabilizer = BasicStabilizer {
        alpha: 0.6,
        decay: 0.95,
        ..BasicStabilizer::default()
    };
    let config = EngineConfig {
        activation_threshold: 0.1,
        routing_policy: RoutingPolicy::default(),
    };
    let stimuli = merge_stimuli(&[Stimulus {
        target: graph_core::EntityId(3),
        signal: SignalVector::new(vec![0.5]),
    }]);

    let mut group = c.benchmark_group("runtime_internal");
    group.bench_function("plan_channel_dispatch_pairwise", |b| {
        b.iter(|| {
            black_box(plan_channel_dispatch_target_count(
                snapshot,
                graph_core::ChannelId(1),
                &config.routing_policy,
                16,
            ))
        })
    });
    group.bench_function("plan_channel_dispatch_broadcast", |b| {
        b.iter(|| {
            black_box(plan_channel_dispatch_target_count(
                snapshot,
                graph_core::ChannelId(2),
                &config.routing_policy,
                16,
            ))
        })
    });
    group.bench_function("plan_channel_dispatch_field", |b| {
        b.iter(|| {
            black_box(plan_channel_dispatch_target_count(
                snapshot,
                graph_core::ChannelId(3),
                &config.routing_policy,
                16,
            ))
        })
    });
    group.bench_function("dispatch_source_pairwise_and_field", |b| {
        b.iter(|| {
            black_box(dispatch_source_emission_count(
                config,
                &stabilizer,
                graph_core::EntityId(1),
                snapshot,
                &laws,
                &config.routing_policy,
                &programs,
            ))
        })
    });
    group.bench_function("dispatch_source_pairwise_only", |b| {
        b.iter(|| {
            black_box(dispatch_source_emission_count(
                config,
                &stabilizer,
                graph_core::EntityId(1),
                pairwise_only_snapshot,
                &laws,
                &config.routing_policy,
                &programs,
            ))
        })
    });
    group.bench_function("dispatch_source_field_only", |b| {
        b.iter(|| {
            black_box(dispatch_source_emission_count(
                config,
                &stabilizer,
                graph_core::EntityId(1),
                field_only_snapshot,
                &laws,
                &config.routing_policy,
                &programs,
            ))
        })
    });
    group.bench_function("dispatch_source_broadcast", |b| {
        b.iter(|| {
            black_box(dispatch_source_emission_count(
                config,
                &stabilizer,
                graph_core::EntityId(3),
                snapshot,
                &laws,
                &config.routing_policy,
                &programs,
            ))
        })
    });

    let dispatch_result = dispatch_source_emission_count(
        config,
        &stabilizer,
        graph_core::EntityId(1),
        snapshot,
        &laws,
        &config.routing_policy,
        &programs,
    );
    let mut inbox = rustc_hash::FxHashMap::default();
    if dispatch_result > 0 {
        inbox.insert(
            graph_core::EntityId(2),
            smallvec::smallvec![Emission {
                signal: SignalVector::new(vec![1.0]),
                magnitude: 1.0,
                cause: graph_core::CauseId(1),
                origin: Some(graph_core::EmissionOrigin {
                    source: graph_core::EntityId(1),
                    target: graph_core::EntityId(2),
                    channel: graph_core::ChannelId(1),
                    law: LawId(1),
                    kind: InteractionKind::Excitatory,
                }),
            }],
        );
    }

    group.bench_function("compute_state_update", |b| {
        b.iter(|| {
            black_box(compute_state_update_present(
                graph_core::EntityId(2),
                snapshot,
                &programs,
                &inbox,
                &rustc_hash::FxHashMap::default(),
                &stimuli,
                &stabilizer,
            ))
        })
    });

    let cohort_emissions = vec![
        Emission {
            signal: SignalVector::new(vec![1.0]),
            magnitude: 1.0,
            cause: graph_core::CauseId(1),
            origin: Some(graph_core::EmissionOrigin {
                source: graph_core::EntityId(3),
                target: graph_core::EntityId(1),
                channel: graph_core::ChannelId(2),
                law: LawId(1),
                kind: InteractionKind::Excitatory,
            }),
        },
        Emission {
            signal: SignalVector::new(vec![1.0]),
            magnitude: 1.0,
            cause: graph_core::CauseId(2),
            origin: Some(graph_core::EmissionOrigin {
                source: graph_core::EntityId(3),
                target: graph_core::EntityId(2),
                channel: graph_core::ChannelId(2),
                law: LawId(1),
                kind: InteractionKind::Excitatory,
            }),
        },
    ];

    group.bench_function("aggregate_cohort_emissions", |b| {
        b.iter(|| black_box(aggregate_cohort_emissions(snapshot, &cohort_emissions)))
    });
    group.finish();
}

criterion_group!(benches, internal_runtime_benchmarks);
criterion_main!(benches);
