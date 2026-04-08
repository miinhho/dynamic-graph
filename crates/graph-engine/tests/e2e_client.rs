use std::cell::Cell;

use graph_core::{
    Channel, ChannelId, EmissionLaw, EntityId, EntityKindId, EntityProgram, EntityState,
    InteractionKind, LawId, SignalVector, StateVector, Stimulus, TickId, WorldVersion,
};
use graph_engine::{
    BasicStabilizer, Engine, EngineConfig, LawRegistry, ProgramRegistry, RetryPolicy,
    RoutingPolicy, RuntimeClient, RuntimeCoordinator, RuntimeTick, TickDiagnostics, TickDriver,
    TickResult,
};
use graph_testkit::{dynamic_channel_world, pairwise_world};
use graph_world::{EntitySelector, SelectorMode, World};

struct LinearProgram;
struct ExcitatoryLaw;
struct FlakyDriver {
    calls: Cell<usize>,
}

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

impl TickDriver for FlakyDriver {
    fn tick<P, L>(
        &self,
        tick: TickId,
        world: &mut World,
        _programs: &P,
        _laws: &L,
        _stimuli: &[Stimulus],
    ) -> TickResult
    where
        P: graph_engine::ProgramCatalog,
        L: graph_engine::LawCatalog,
    {
        let call_index = self.calls.get();
        self.calls.set(call_index + 1);

        let mut transaction = graph_tx::TickTransaction::simulate(tick, world.version());
        if call_index == 0 {
            transaction.mark_conflict(world.version(), WorldVersion(world.version().0 + 1));
        } else {
            transaction.mark_committed(world.version());
        }

        TickResult {
            diagnostics: TickDiagnostics {
                tick,
                ..TickDiagnostics::default()
            },
            transaction,
        }
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

    let mut laws = LawRegistry::default();
    laws.insert(LawId(1), Box::new(ExcitatoryLaw));

    (programs, laws)
}

#[test]
fn runtime_client_e2e_runs_engine_and_reads_results() {
    let mut world = pairwise_world();
    let engine = engine();
    let (programs, laws) = registries();
    let client = RuntimeClient::new(&engine, &programs, &laws);

    let before = client.read(&world);
    assert_eq!(before.entities().count(), 2);
    assert_eq!(before.channels().from(EntityId(1)).ids(), &[ChannelId(1)]);

    let result = client.tick(
        TickId(1),
        &mut world,
        &[Stimulus {
            target: EntityId(1),
            signal: SignalVector::new(vec![0.5]),
        }],
    );

    let after = client.inspect(&world, &result);
    assert_eq!(after.deltas().len(), 2);
    assert_eq!(after.changed_entity_projections().len(), 2);
    assert_eq!(after.channels().to(EntityId(2)).ids(), &[ChannelId(1)]);

    let target_projection = after
        .entities()
        .kind(EntityKindId(2))
        .projections()
        .into_iter()
        .next()
        .expect("target entity projection should exist");
    assert!(target_projection.internal_norm() > 0.0);
    assert!(
        after
            .inspection()
            .transaction_summary()
            .committed_version
            .is_some()
    );
}

#[test]
fn runtime_client_e2e_handles_coordinator_retry_with_same_surface() {
    let mut world = pairwise_world();
    let (programs, laws) = registries();
    let coordinator = RuntimeCoordinator::new(
        FlakyDriver {
            calls: Cell::new(0),
        },
        RetryPolicy { max_attempts: 3 },
    );
    let client = RuntimeClient::new(&coordinator, &programs, &laws);

    let result: RuntimeTick = client.tick(TickId(9), &mut world, &[]);
    let read = client.inspect(&world, &result);

    assert_eq!(result.attempts, 2);
    assert!(!read.inspection().transaction_summary().has_conflict);
    assert_eq!(read.entities().count(), 2);
}

#[test]
fn runtime_client_e2e_supports_query_and_selector_reads() {
    let world = dynamic_channel_world();
    let engine = engine();
    let (programs, laws) = registries();
    let client = RuntimeClient::new(&engine, &programs, &laws);

    let read = client.read(&world);

    assert_eq!(read.entities().kind(EntityKindId(1)).count(), 2);
    assert_eq!(
        read.channels().from(EntityId(1)).ids(),
        &[ChannelId(1), ChannelId(3)]
    );
    assert_eq!(
        read.channels().to(EntityId(2)).ids(),
        &[ChannelId(1), ChannelId(2), ChannelId(3)]
    );

    let selected = read
        .entities()
        .select(&EntitySelector {
            source: EntityId(1),
            targets: Vec::new(),
            target_kinds: vec![EntityKindId(1)],
            radius: Some(2.0),
            mode: SelectorMode::IndexedOnly,
        })
        .ids()
        .to_vec();
    assert_eq!(selected, vec![EntityId(2)]);

    let indexed_empty = read.query().select(&EntitySelector {
        source: EntityId(1),
        targets: Vec::new(),
        target_kinds: Vec::new(),
        radius: None,
        mode: SelectorMode::IndexedOnly,
    });
    assert!(indexed_empty.targets.is_empty());

    let full_scan = read.query().select(&EntitySelector {
        source: EntityId(1),
        targets: Vec::new(),
        target_kinds: Vec::new(),
        radius: None,
        mode: SelectorMode::AllowFullScan,
    });
    assert_eq!(
        full_scan.targets,
        vec![EntityId(2), EntityId(3), EntityId(4)]
    );
}

#[test]
fn runtime_client_e2e_exposes_state_and_delta_views_after_tick() {
    let mut world = dynamic_channel_world();
    let engine = engine();
    let (programs, laws) = registries();
    let client = RuntimeClient::new(&engine, &programs, &laws);

    let result = client.tick(
        TickId(3),
        &mut world,
        &[Stimulus {
            target: EntityId(3),
            signal: SignalVector::new(vec![0.5]),
        }],
    );
    let read = client.inspect(&world, &result);

    assert_eq!(read.inspection().transaction_summary().tick, TickId(3));
    assert!(
        read.inspection()
            .transaction_summary()
            .committed_version
            .is_some()
    );
    assert!(read.deltas().len() >= 2);
    assert!(!read.changed_entity_projections().is_empty());
    assert_eq!(
        read.channels().to(EntityId(2)).ids(),
        &[ChannelId(1), ChannelId(2), ChannelId(3)]
    );

    let delta_entities = read
        .deltas()
        .iter()
        .map(|delta| delta.entity)
        .collect::<Vec<_>>();
    assert!(delta_entities.contains(&EntityId(2)));
    assert_eq!(
        read.inspection()
            .delta_by_cause(graph_core::CauseId(1))
            .map(|delta| delta.cause),
        Some(graph_core::CauseId(1))
    );
}
