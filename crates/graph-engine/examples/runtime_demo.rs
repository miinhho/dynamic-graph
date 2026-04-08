use graph_core::{
    Channel, ChannelId, ChannelMode, CohortReducer, EmissionBudget, EmissionLaw, Entity, EntityId,
    EntityKindId, EntityProgram, EntityState, FieldKernel, InteractionKind, LawId, SignalVector,
    StateVector, Stimulus, TickId,
};
use graph_engine::{
    BasicStabilizer, Engine, EngineConfig, LawRegistry, ProgramRegistry, RoutingPolicy,
};
use graph_world::World;

struct LinearProgram;

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

struct ExcitatoryLaw;
struct InhibitoryLaw;

impl EmissionLaw for ExcitatoryLaw {
    fn project(&self, outbound: &SignalVector, _: &EntityState, channel: &Channel) -> SignalVector {
        outbound.scaled(channel.weight)
    }

    fn kind(&self) -> InteractionKind {
        InteractionKind::Excitatory
    }
}

impl EmissionLaw for InhibitoryLaw {
    fn project(&self, outbound: &SignalVector, _: &EntityState, channel: &Channel) -> SignalVector {
        outbound.scaled(-channel.weight.abs())
    }

    fn kind(&self) -> InteractionKind {
        InteractionKind::Inhibitory
    }
}

fn main() {
    let mut world = World::default();
    world.insert_entity(Entity {
        id: EntityId(1),
        kind: EntityKindId(1),
        position: StateVector::new(vec![0.0]),
        state: EntityState {
            internal: StateVector::new(vec![1.0]),
            emitted: SignalVector::new(vec![1.0]),
            cooldown: 0,
        },
        refractory_period: 1,
        budget: EmissionBudget {
            max_targets_per_tick: 2,
            max_signal_norm: 2.0,
        },
    });
    for id in 2..=4 {
        world.insert_entity(Entity {
            id: EntityId(id),
            kind: EntityKindId(1),
            position: StateVector::new(vec![id as f32]),
            state: EntityState::default(),
            refractory_period: 0,
            budget: EmissionBudget::default(),
        });
    }
    world.insert_channel(Channel {
        id: ChannelId(1),
        source: EntityId(1),
        targets: vec![EntityId(2), EntityId(3), EntityId(4)],
        target_kinds: Vec::new(),
        field_radius: None,
        field_kernel: FieldKernel::Flat,
        cohort_reducer: CohortReducer::Sum,
        law: LawId(1),
        kind: ChannelMode::Broadcast,
        weight: 1.0,
        attenuation: 0.0,
        enabled: true,
    });
    world.insert_channel(Channel {
        id: ChannelId(2),
        source: EntityId(3),
        targets: vec![EntityId(2)],
        target_kinds: Vec::new(),
        field_radius: None,
        field_kernel: FieldKernel::Flat,
        cohort_reducer: CohortReducer::Sum,
        law: LawId(2),
        kind: ChannelMode::Pairwise,
        weight: 0.7,
        attenuation: 0.0,
        enabled: true,
    });

    let mut programs = ProgramRegistry::default();
    programs.insert(EntityKindId(1), Box::new(LinearProgram));

    let mut laws = LawRegistry::default();
    laws.insert(LawId(1), Box::new(ExcitatoryLaw));
    laws.insert(LawId(2), Box::new(InhibitoryLaw));

    let engine = Engine::new(
        EngineConfig {
            activation_threshold: 0.1,
            routing_policy: RoutingPolicy {
                field_threshold: 3,
                cohort_threshold: usize::MAX,
            },
        },
        BasicStabilizer {
            alpha: 0.6,
            decay: 0.95,
            ..BasicStabilizer::default()
        },
    );

    for tick in 1..=4 {
        let external = if tick == 2 {
            vec![Stimulus {
                target: EntityId(1),
                signal: SignalVector::new(vec![1.0]),
            }]
        } else {
            Vec::new()
        };
        let result = engine.tick(TickId(tick), &mut world, &programs, &laws, &external);
        println!(
            "tick={} active={:?} capped={:?} emissions={}",
            tick,
            result
                .diagnostics
                .active_entities
                .iter()
                .map(|id| id.0)
                .collect::<Vec<_>>(),
            result
                .diagnostics
                .fanout_capped_entities
                .iter()
                .map(|id| id.0)
                .collect::<Vec<_>>(),
            result.diagnostics.total_emissions
        );
        for entity in world.entities() {
            println!(
                "  entity={} internal={:.3} emitted={:.3} cooldown={}",
                entity.id.0,
                entity.state.internal.first().unwrap_or_default(),
                entity.state.emitted.first().unwrap_or_default(),
                entity.state.cooldown
            );
        }
    }
}
