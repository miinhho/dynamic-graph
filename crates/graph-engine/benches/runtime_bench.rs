use criterion::{BatchSize, Criterion, black_box, criterion_group, criterion_main};
use graph_core::{
    Channel, EmissionLaw, EntityKindId, EntityProgram, EntityState, InteractionKind, LawId,
    SignalVector, StateVector, Stimulus, TickId,
};
use graph_engine::{
    BasicStabilizer, Engine, EngineConfig, LawRegistry, ProgramRegistry, RoutingPolicy,
    RuntimeClient,
};
use graph_testkit::{dynamic_channel_world, pairwise_world, representative_runtime_world};

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

fn engine() -> Engine<BasicStabilizer> {
    Engine::new(
        EngineConfig {
            activation_threshold: 0.1,
            routing_policy: RoutingPolicy::default(),
        },
        BasicStabilizer {
            alpha: 0.6,
            decay: 0.95,
            ..BasicStabilizer::default()
        },
    )
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

fn runtime_benchmarks(c: &mut Criterion) {
    let engine = engine();
    let (programs, laws) = registries();
    let client = RuntimeClient::new(&engine, &programs, &laws);

    let pairwise = pairwise_world();
    let dynamic = dynamic_channel_world();
    let medium = representative_runtime_world(64);
    let large = representative_runtime_world(256);
    let stimuli = [Stimulus {
        target: graph_core::EntityId(1),
        signal: SignalVector::new(vec![0.5]),
    }];

    let mut group = c.benchmark_group("runtime_tick");
    group.bench_function("pairwise_tick", |b| {
        b.iter_batched(
            || pairwise.clone(),
            |mut world| {
                black_box(client.tick(TickId(1), &mut world, &stimuli));
            },
            BatchSize::SmallInput,
        )
    });
    group.bench_function("dynamic_tick", |b| {
        b.iter_batched(
            || dynamic.clone(),
            |mut world| {
                black_box(client.tick(TickId(1), &mut world, &stimuli));
            },
            BatchSize::SmallInput,
        )
    });
    group.bench_function("representative_tick_64", |b| {
        b.iter_batched(
            || medium.clone(),
            |mut world| {
                black_box(client.tick(TickId(1), &mut world, &stimuli));
            },
            BatchSize::SmallInput,
        )
    });
    group.bench_function("representative_tick_256", |b| {
        b.iter_batched(
            || large.clone(),
            |mut world| {
                black_box(client.tick(TickId(1), &mut world, &stimuli));
            },
            BatchSize::SmallInput,
        )
    });
    group.finish();
}

criterion_group!(benches, runtime_benchmarks);
criterion_main!(benches);
