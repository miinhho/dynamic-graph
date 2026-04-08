use graph_core::{
    Channel, ChannelId, ChannelMode, CohortReducer, EmissionBudget, Entity, EntityId, EntityKindId,
    EntityState, FieldKernel, LawId, SignalVector, StateVector,
};
use graph_world::World;

pub fn entity(id: u64, kind: u64, position: f32) -> Entity {
    Entity {
        id: EntityId(id),
        kind: EntityKindId(kind),
        position: StateVector::new(vec![position]),
        state: EntityState {
            internal: StateVector::default(),
            emitted: SignalVector::default(),
            cooldown: 0,
        },
        refractory_period: 0,
        budget: EmissionBudget::default(),
    }
}

pub fn emitting_entity(id: u64, kind: u64, position: f32, emitted: f32) -> Entity {
    Entity {
        state: EntityState {
            emitted: SignalVector::new(vec![emitted]),
            ..entity(id, kind, position).state
        },
        ..entity(id, kind, position)
    }
}

pub fn pairwise_channel(id: u64, source: u64, target: u64, law: u64) -> Channel {
    Channel {
        id: ChannelId(id),
        source: EntityId(source),
        targets: vec![EntityId(target)],
        target_kinds: Vec::new(),
        field_radius: None,
        field_kernel: FieldKernel::Flat,
        cohort_reducer: CohortReducer::Sum,
        law: LawId(law),
        kind: ChannelMode::Pairwise,
        weight: 1.0,
        attenuation: 0.0,
        enabled: true,
    }
}

pub fn broadcast_channel(id: u64, source: u64, target_kind: u64, radius: f32, law: u64) -> Channel {
    Channel {
        id: ChannelId(id),
        source: EntityId(source),
        targets: Vec::new(),
        target_kinds: vec![EntityKindId(target_kind)],
        field_radius: Some(radius),
        field_kernel: FieldKernel::Flat,
        cohort_reducer: CohortReducer::Sum,
        law: LawId(law),
        kind: ChannelMode::Broadcast,
        weight: 1.0,
        attenuation: 0.0,
        enabled: true,
    }
}

pub fn field_channel(id: u64, source: u64, radius: f32, law: u64) -> Channel {
    Channel {
        id: ChannelId(id),
        source: EntityId(source),
        targets: Vec::new(),
        target_kinds: Vec::new(),
        field_radius: Some(radius),
        field_kernel: FieldKernel::Flat,
        cohort_reducer: CohortReducer::Sum,
        law: LawId(law),
        kind: ChannelMode::Field,
        weight: 1.0,
        attenuation: 0.0,
        enabled: true,
    }
}

pub fn single_agent_world() -> World {
    world_with_entities([entity(1, 1, 0.0)])
}

pub fn world_with_entities<const N: usize>(entities: [Entity; N]) -> World {
    let mut world = World::default();
    for entity in entities {
        world.insert_entity(entity);
    }
    world
}

pub fn world_from_entities<I>(entities: I) -> World
where
    I: IntoIterator<Item = Entity>,
{
    let mut world = World::default();
    for entity in entities {
        world.insert_entity(entity);
    }
    world
}

pub fn world_with_components<const E: usize, const C: usize>(
    entities: [Entity; E],
    channels: [Channel; C],
) -> World {
    let mut world = world_with_entities(entities);
    for channel in channels {
        world.insert_channel(channel);
    }
    world
}

pub fn world_from_components<E, C>(entities: E, channels: C) -> World
where
    E: IntoIterator<Item = Entity>,
    C: IntoIterator<Item = Channel>,
{
    let mut world = world_from_entities(entities);
    for channel in channels {
        world.insert_channel(channel);
    }
    world
}

pub fn pairwise_world() -> World {
    world_with_components(
        [emitting_entity(1, 1, 0.0, 1.0), entity(2, 2, 1.0)],
        [pairwise_channel(1, 1, 2, 1)],
    )
}

pub fn dynamic_channel_world() -> World {
    world_with_components(
        [
            emitting_entity(1, 1, 0.0, 1.0),
            entity(2, 1, 1.0),
            entity(3, 2, 3.0),
            entity(4, 3, 0.5),
        ],
        [
            pairwise_channel(1, 1, 2, 1),
            broadcast_channel(2, 3, 1, 5.0, 1),
            field_channel(3, 1, 2.0, 1),
        ],
    )
}

/// Two entities with mutual pairwise channels — the smallest world that
/// exhibits a real cycle for SCC tests and oscillation/stability fixtures.
pub fn cyclic_pair_world() -> World {
    world_with_components(
        [
            emitting_entity(1, 1, 0.0, 1.0),
            emitting_entity(2, 1, 1.0, 1.0),
        ],
        [
            pairwise_channel(1, 1, 2, 1),
            pairwise_channel(2, 2, 1, 1),
        ],
    )
}

/// Linear chain of `n` entities (`n >= 2`) connected by pairwise channels
/// from `i` to `i+1`. The first entity is emitting so a tick stream actually
/// propagates a signal down the chain.
pub fn chain_world(n: u64) -> World {
    let n = n.max(2);
    let mut entities = Vec::with_capacity(n as usize);
    entities.push(emitting_entity(1, 1, 0.0, 1.0));
    for id in 2..=n {
        entities.push(entity(id, 1, (id - 1) as f32));
    }
    let mut channels = Vec::with_capacity((n - 1) as usize);
    for id in 1..n {
        channels.push(pairwise_channel(id, id, id + 1, 1));
    }
    world_from_components(entities, channels)
}

/// A signed-cycle world where two entities form a cycle and a third entity
/// receives a contradictory pairwise edge from each. Designed for frustration
/// stress tests rather than convergence tests.
pub fn signed_cycle_world() -> World {
    world_with_components(
        [
            emitting_entity(1, 1, 0.0, 1.0),
            emitting_entity(2, 1, 1.0, -1.0),
            entity(3, 2, 2.0),
        ],
        [
            pairwise_channel(1, 1, 2, 1),
            pairwise_channel(2, 2, 1, 1),
            pairwise_channel(3, 1, 3, 1),
            pairwise_channel(4, 2, 3, 1),
        ],
    )
}

pub fn representative_runtime_world(entity_count: u64) -> World {
    let entity_count = entity_count.max(8);
    let mut entities = Vec::with_capacity(entity_count as usize);
    let mut channels = Vec::new();

    for id in 1..=entity_count {
        let kind = match id % 3 {
            0 => 3,
            1 => 1,
            _ => 2,
        };
        let position = (id - 1) as f32 * 0.75;
        let base = if kind == 1 && id % 4 == 1 {
            emitting_entity(id, kind, position, 1.0)
        } else {
            entity(id, kind, position)
        };
        entities.push(base);
    }

    let mut channel_id = 1_u64;
    for id in 1..entity_count {
        channels.push(pairwise_channel(channel_id, id, id + 1, 1));
        channel_id += 1;
    }

    for id in (1..=entity_count).step_by(4) {
        let target_kind = if id % 2 == 0 { 2 } else { 1 };
        channels.push(broadcast_channel(channel_id, id, target_kind, 3.0, 1));
        channel_id += 1;
    }

    for id in (1..=entity_count).step_by(5) {
        channels.push(field_channel(channel_id, id, 2.5, 1));
        channel_id += 1;
    }

    world_from_components(entities, channels)
}

pub fn representative_query_world(entity_count: u64) -> World {
    let entity_count = entity_count.max(16);
    let mut entities = Vec::with_capacity(entity_count as usize);
    let mut channels = Vec::new();

    for id in 1..=entity_count {
        let kind = if id % 2 == 0 { 1 } else { 2 };
        entities.push(entity(id, kind, (id - 1) as f32 * 0.5));
    }

    let mut channel_id = 1_u64;
    for id in 1..entity_count {
        channels.push(pairwise_channel(channel_id, id, id + 1, 1));
        channel_id += 1;
    }
    for id in (1..=entity_count).step_by(6) {
        channels.push(broadcast_channel(channel_id, id, 1, 4.0, 1));
        channel_id += 1;
    }
    for id in (1..=entity_count).step_by(7) {
        channels.push(field_channel(channel_id, id, 3.0, 1));
        channel_id += 1;
    }

    world_from_components(entities, channels)
}
