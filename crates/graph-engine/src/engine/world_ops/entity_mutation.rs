use graph_core::{
    BatchId, EmergenceProposal, Entity, EntityLayer, EntityLineage, EntitySnapshot, EntityStatus,
    EntityWeatheringPolicy, LayerTransition, LifecycleCause, RelationshipId, WeatheringEffect,
    WorldEvent, apply_skeleton,
};
use graph_world::World;

struct ProposalRuntime<'a> {
    world: &'a mut World,
    batch: BatchId,
    events: &'a mut Vec<WorldEvent>,
}

struct EntityWeatheringPlan {
    entity_id: graph_core::EntityId,
    actions: Vec<LayerWeatheringAction>,
}

struct EntityMutationEffect {
    entity_id: graph_core::EntityId,
    status: Option<EntityStatus>,
    current: Option<EntitySnapshot>,
    layer: EntityLayer,
    lineage_children_to_add: Vec<graph_core::EntityId>,
    event: Option<WorldEvent>,
}

struct NewEntityEffect {
    entity: Entity,
    event: WorldEvent,
}

#[derive(Clone, Copy)]
enum LayerWeatheringAction {
    Preserve,
    Compress,
    Skeletonize,
    Remove,
}

pub(crate) fn apply_proposals(
    world: &mut World,
    proposals: Vec<EmergenceProposal>,
    batch: BatchId,
) -> Vec<WorldEvent> {
    let mut events = Vec::new();

    for proposal in proposals {
        apply_proposal(world, batch, &mut events, proposal);
    }

    events
}

pub(crate) fn weather_entities(world: &mut World, policy: &dyn EntityWeatheringPolicy) {
    let current_batch = world.current_batch().0;
    let plans = build_weathering_plans(world, current_batch, policy);
    apply_weathering_plans(world, &plans);
}

fn apply_proposal(
    world: &mut World,
    batch: BatchId,
    events: &mut Vec<WorldEvent>,
    proposal: EmergenceProposal,
) {
    let mut runtime = ProposalRuntime {
        world,
        batch,
        events,
    };
    match proposal {
        EmergenceProposal::Born {
            members,
            member_relationships,
            coherence,
            parents,
            cause,
        } => apply_born_proposal(
            &mut runtime,
            members,
            member_relationships,
            coherence,
            parents,
            cause,
        ),
        EmergenceProposal::DepositLayer { entity, layer } => {
            apply_state_transition_proposal(&mut runtime, StateTransitionProposal::Deposit {
                entity,
                layer,
            })
        }
        EmergenceProposal::Dormant { entity, cause } => {
            apply_state_transition_proposal(&mut runtime, StateTransitionProposal::Dormant {
                entity,
                cause,
            })
        }
        EmergenceProposal::Revive {
            entity,
            snapshot,
            cause,
        } => apply_state_transition_proposal(&mut runtime, StateTransitionProposal::Revive {
            entity,
            snapshot,
            cause,
        }),
        EmergenceProposal::Split {
            source,
            offspring,
            cause,
        } => apply_lineage_proposal(&mut runtime, LineageProposal::Split {
            source,
            offspring,
            cause,
        }),
        EmergenceProposal::Merge {
            absorbed,
            into,
            new_members,
            member_relationships,
            coherence,
            cause,
        } => apply_lineage_proposal(&mut runtime, LineageProposal::Merge {
            absorbed,
            into,
            new_members,
            member_relationships,
            coherence,
            cause,
        }),
    }
}

enum StateTransitionProposal {
    Deposit {
        entity: graph_core::EntityId,
        layer: EntityLayer,
    },
    Dormant {
        entity: graph_core::EntityId,
        cause: LifecycleCause,
    },
    Revive {
        entity: graph_core::EntityId,
        snapshot: EntitySnapshot,
        cause: LifecycleCause,
    },
}

fn apply_state_transition_proposal(
    runtime: &mut ProposalRuntime<'_>,
    proposal: StateTransitionProposal,
) {
    match proposal {
        StateTransitionProposal::Deposit { entity, layer } => apply_deposit_layer_proposal(
            runtime.world,
            runtime.batch,
            runtime.events,
            entity,
            layer,
        ),
        StateTransitionProposal::Dormant { entity, cause } => {
            apply_dormant_proposal(runtime.world, runtime.batch, runtime.events, entity, cause)
        }
        StateTransitionProposal::Revive {
            entity,
            snapshot,
            cause,
        } => apply_revive_proposal(
            runtime.world,
            runtime.batch,
            runtime.events,
            entity,
            snapshot,
            cause,
        ),
    }
}

enum LineageProposal {
    Split {
        source: graph_core::EntityId,
        offspring: Vec<(Vec<graph_core::LocusId>, Vec<RelationshipId>, f32)>,
        cause: LifecycleCause,
    },
    Merge {
        absorbed: Vec<graph_core::EntityId>,
        into: graph_core::EntityId,
        new_members: Vec<graph_core::LocusId>,
        member_relationships: Vec<RelationshipId>,
        coherence: f32,
        cause: LifecycleCause,
    },
}

fn apply_lineage_proposal(runtime: &mut ProposalRuntime<'_>, proposal: LineageProposal) {
    match proposal {
        LineageProposal::Split {
            source,
            offspring,
            cause,
        } => apply_split_proposal(
            runtime.world,
            runtime.batch,
            runtime.events,
            source,
            offspring,
            cause,
        ),
        LineageProposal::Merge {
            absorbed,
            into,
            new_members,
            member_relationships,
            coherence,
            cause,
        } => apply_merge_proposal(
            runtime,
            absorbed,
            into,
            new_members,
            member_relationships,
            coherence,
            cause,
        ),
    }
}

fn apply_born_proposal(
    runtime: &mut ProposalRuntime<'_>,
    members: Vec<graph_core::LocusId>,
    member_relationships: Vec<RelationshipId>,
    coherence: f32,
    parents: Vec<graph_core::EntityId>,
    cause: LifecycleCause,
) {
    let store = runtime.world.entities_mut();
    let id = store.mint_id();
    let effect = build_born_entity_effect(
        id,
        runtime.batch,
        members,
        member_relationships,
        coherence,
        parents,
        cause,
    );
    apply_new_entity_effect(runtime.world, runtime.events, effect);
}

fn apply_deposit_layer_proposal(
    world: &mut World,
    batch: BatchId,
    events: &mut Vec<WorldEvent>,
    entity: graph_core::EntityId,
    layer: EntityLayer,
) {
    let effect = build_deposit_layer_effect(batch, entity, layer);
    apply_entity_mutation_effect(world, events, effect);
}

fn apply_dormant_proposal(
    world: &mut World,
    batch: BatchId,
    events: &mut Vec<WorldEvent>,
    entity: graph_core::EntityId,
    cause: LifecycleCause,
) {
    if let Some(effect) = build_dormant_effect(world, batch, entity, cause) {
        apply_entity_mutation_effect(world, events, effect);
    }
}

fn apply_revive_proposal(
    world: &mut World,
    batch: BatchId,
    events: &mut Vec<WorldEvent>,
    entity: graph_core::EntityId,
    snapshot: EntitySnapshot,
    cause: LifecycleCause,
) {
    let effect = build_revive_effect(batch, entity, snapshot, cause);
    apply_entity_mutation_effect(world, events, effect);
}

fn apply_split_proposal(
    world: &mut World,
    batch: BatchId,
    events: &mut Vec<WorldEvent>,
    source: graph_core::EntityId,
    offspring: Vec<(Vec<graph_core::LocusId>, Vec<RelationshipId>, f32)>,
    cause: LifecycleCause,
) {
    let mut child_ids = Vec::new();
    for (members, member_relationships, coherence) in offspring {
        let store = world.entities_mut();
        let child_id = store.mint_id();
        let effect = build_born_entity_effect(
            child_id,
            batch,
            members,
            member_relationships,
            coherence,
            Vec::new(),
            cause.clone(),
        );
        apply_new_entity_effect(world, events, effect);
        child_ids.push(child_id);
    }
    if let Some(effect) = build_split_source_effect(world, batch, source, &child_ids, cause) {
        apply_entity_mutation_effect(world, events, effect);
    }
    events.push(entity_split_event(source, child_ids, batch));
}

fn apply_merge_proposal(
    runtime: &mut ProposalRuntime<'_>,
    absorbed: Vec<graph_core::EntityId>,
    into: graph_core::EntityId,
    new_members: Vec<graph_core::LocusId>,
    member_relationships: Vec<RelationshipId>,
    coherence: f32,
    cause: LifecycleCause,
) {
    for absorbed_id in &absorbed {
        if let Some(effect) =
            build_absorbed_merge_effect(runtime.world, runtime.batch, *absorbed_id, into)
        {
            apply_entity_mutation_effect(runtime.world, runtime.events, effect);
        }
    }
    let snapshot = entity_snapshot(new_members, member_relationships, coherence);
    let effect =
        build_survivor_merge_effect(runtime.batch, into, snapshot, absorbed.clone(), cause);
    apply_entity_mutation_effect(runtime.world, runtime.events, effect);
    runtime
        .events
        .push(entity_merged_event(absorbed, into, runtime.batch));
}

fn build_weathering_plans(
    world: &World,
    current_batch: u64,
    policy: &dyn EntityWeatheringPolicy,
) -> Vec<EntityWeatheringPlan> {
    world
        .entities()
        .iter()
        .map(|entity| build_entity_weathering_plan(entity, current_batch, policy))
        .collect()
}

fn build_entity_weathering_plan(
    entity: &Entity,
    current_batch: u64,
    policy: &dyn EntityWeatheringPolicy,
) -> EntityWeatheringPlan {
    let actions = entity
        .layers
        .iter()
        .map(|layer| {
            let age = current_batch.saturating_sub(layer.batch.0);
            plan_weathering_action(layer, policy.effect(layer, age))
        })
        .collect();

    EntityWeatheringPlan {
        entity_id: entity.id,
        actions,
    }
}

fn plan_weathering_action(layer: &EntityLayer, effect: WeatheringEffect) -> LayerWeatheringAction {
    match effect {
        WeatheringEffect::Preserved => LayerWeatheringAction::Preserve,
        WeatheringEffect::Compress => LayerWeatheringAction::Compress,
        WeatheringEffect::Skeleton => LayerWeatheringAction::Skeletonize,
        WeatheringEffect::Remove => {
            if layer.transition.is_significant() {
                LayerWeatheringAction::Skeletonize
            } else {
                LayerWeatheringAction::Remove
            }
        }
    }
}

fn apply_weathering_plans(world: &mut World, plans: &[EntityWeatheringPlan]) {
    for plan in plans {
        if let Some(entity) = world.entities_mut().get_mut(plan.entity_id) {
            apply_entity_weathering_plan(entity, plan);
        }
    }
}

fn apply_entity_weathering_plan(entity: &mut Entity, plan: &EntityWeatheringPlan) {
    let mut i = 0;
    for action in &plan.actions {
        if i >= entity.layers.len() {
            break;
        }
        let step = apply_weathering_action(entity, i, *action);
        i += step;
    }
}

fn apply_weathering_action(
    entity: &mut Entity,
    layer_index: usize,
    action: LayerWeatheringAction,
) -> usize {
    match action {
        LayerWeatheringAction::Preserve => 1,
        LayerWeatheringAction::Compress => {
            graph_core::apply_compress(&mut entity.layers[layer_index]);
            1
        }
        LayerWeatheringAction::Skeletonize => {
            apply_skeleton(&mut entity.layers[layer_index]);
            1
        }
        LayerWeatheringAction::Remove => {
            entity.layers.remove(layer_index);
            0
        }
    }
}

fn build_born_entity_effect(
    id: graph_core::EntityId,
    batch: BatchId,
    members: Vec<graph_core::LocusId>,
    member_relationships: Vec<RelationshipId>,
    coherence: f32,
    parents: Vec<graph_core::EntityId>,
    cause: LifecycleCause,
) -> NewEntityEffect {
    let member_count = members.len();
    let snapshot = entity_snapshot(members, member_relationships, coherence);
    let mut entity = Entity::born(id, batch, snapshot);
    if let Some(layer) = entity.layers.last_mut() {
        layer.cause = cause;
    }
    entity.lineage = EntityLineage {
        parents,
        children: Vec::new(),
    };
    NewEntityEffect {
        entity,
        event: entity_born_event(id, batch, member_count),
    }
}

fn build_split_source_effect(
    world: &World,
    batch: BatchId,
    source: graph_core::EntityId,
    child_ids: &[graph_core::EntityId],
    cause: LifecycleCause,
) -> Option<EntityMutationEffect> {
    let entity = world.entities().get(source)?;
    Some(EntityMutationEffect {
        entity_id: source,
        status: Some(EntityStatus::Dormant),
        current: None,
        layer: entity_layer(
            batch,
            entity.current.clone(),
            LayerTransition::Split {
                offspring: child_ids.to_vec(),
            },
            cause,
        ),
        lineage_children_to_add: child_ids.to_vec(),
        event: None,
    })
}

fn build_absorbed_merge_effect(
    world: &World,
    batch: BatchId,
    absorbed_id: graph_core::EntityId,
    into: graph_core::EntityId,
) -> Option<EntityMutationEffect> {
    let entity = world.entities().get(absorbed_id)?;
    Some(EntityMutationEffect {
        entity_id: absorbed_id,
        status: Some(EntityStatus::Dormant),
        current: None,
        layer: entity_layer(
            batch,
            entity.current.clone(),
            LayerTransition::Merged {
                absorbed: vec![into],
            },
            LifecycleCause::MergedInto { survivor: into },
        ),
        lineage_children_to_add: Vec::new(),
        event: None,
    })
}

fn build_survivor_merge_effect(
    batch: BatchId,
    into: graph_core::EntityId,
    snapshot: EntitySnapshot,
    absorbed: Vec<graph_core::EntityId>,
    cause: LifecycleCause,
) -> EntityMutationEffect {
    EntityMutationEffect {
        entity_id: into,
        status: None,
        current: Some(snapshot.clone()),
        layer: entity_layer(
            batch,
            snapshot,
            LayerTransition::Merged {
                absorbed: absorbed.clone(),
            },
            cause,
        ),
        lineage_children_to_add: absorbed,
        event: None,
    }
}

fn build_deposit_layer_effect(
    batch: BatchId,
    entity_id: graph_core::EntityId,
    layer: EntityLayer,
) -> EntityMutationEffect {
    let event = coherence_shift_event(entity_id, batch, &layer.transition);
    EntityMutationEffect {
        entity_id,
        status: None,
        current: Some(layer.snapshot.clone().unwrap_or_default()),
        layer,
        lineage_children_to_add: Vec::new(),
        event,
    }
}

fn build_dormant_effect(
    world: &World,
    batch: BatchId,
    entity_id: graph_core::EntityId,
    cause: LifecycleCause,
) -> Option<EntityMutationEffect> {
    let entity = world.entities().get(entity_id)?;
    Some(EntityMutationEffect {
        entity_id,
        status: Some(EntityStatus::Dormant),
        current: None,
        layer: entity_layer(
            batch,
            entity.current.clone(),
            LayerTransition::BecameDormant,
            cause,
        ),
        lineage_children_to_add: Vec::new(),
        event: Some(entity_dormant_event(entity_id, batch)),
    })
}

fn build_revive_effect(
    batch: BatchId,
    entity_id: graph_core::EntityId,
    snapshot: EntitySnapshot,
    cause: LifecycleCause,
) -> EntityMutationEffect {
    EntityMutationEffect {
        entity_id,
        status: Some(EntityStatus::Active),
        current: Some(snapshot.clone()),
        layer: entity_layer(batch, snapshot, LayerTransition::Revived, cause),
        lineage_children_to_add: Vec::new(),
        event: Some(entity_revived_event(entity_id, batch)),
    }
}

fn apply_new_entity_effect(
    world: &mut World,
    events: &mut Vec<WorldEvent>,
    effect: NewEntityEffect,
) {
    world.entities_mut().insert(effect.entity);
    events.push(effect.event);
}

fn apply_entity_mutation_effect(
    world: &mut World,
    events: &mut Vec<WorldEvent>,
    effect: EntityMutationEffect,
) {
    if let Some(entity) = world.entities_mut().get_mut(effect.entity_id) {
        if let Some(status) = effect.status {
            entity.status = status;
        }
        if let Some(current) = effect.current {
            entity.current = current;
        }
        entity
            .lineage
            .children
            .extend(effect.lineage_children_to_add);
        entity.layers.push(effect.layer);
        if let Some(event) = effect.event {
            events.push(event);
        }
    }
}

fn entity_born_event(
    entity: graph_core::EntityId,
    batch: BatchId,
    member_count: usize,
) -> WorldEvent {
    WorldEvent::EntityBorn {
        entity,
        batch,
        member_count,
    }
}

fn entity_snapshot(
    members: Vec<graph_core::LocusId>,
    member_relationships: Vec<RelationshipId>,
    coherence: f32,
) -> EntitySnapshot {
    EntitySnapshot {
        members,
        member_relationships,
        coherence,
    }
}

fn entity_layer(
    batch: BatchId,
    snapshot: EntitySnapshot,
    transition: LayerTransition,
    cause: LifecycleCause,
) -> EntityLayer {
    EntityLayer::new(batch, snapshot, transition).with_cause(cause)
}

fn coherence_shift_event(
    entity: graph_core::EntityId,
    batch: BatchId,
    transition: &LayerTransition,
) -> Option<WorldEvent> {
    match transition {
        LayerTransition::CoherenceShift { from, to } => Some(WorldEvent::CoherenceShift {
            entity,
            from: *from,
            to: *to,
            batch,
        }),
        _ => None,
    }
}

fn entity_dormant_event(entity: graph_core::EntityId, batch: BatchId) -> WorldEvent {
    WorldEvent::EntityDormant { entity, batch }
}

fn entity_revived_event(entity: graph_core::EntityId, batch: BatchId) -> WorldEvent {
    WorldEvent::EntityRevived { entity, batch }
}

fn entity_split_event(
    source: graph_core::EntityId,
    offspring: Vec<graph_core::EntityId>,
    batch: BatchId,
) -> WorldEvent {
    WorldEvent::EntitySplit {
        source,
        offspring,
        batch,
    }
}

fn entity_merged_event(
    absorbed: Vec<graph_core::EntityId>,
    into: graph_core::EntityId,
    batch: BatchId,
) -> WorldEvent {
    WorldEvent::EntityMerged {
        absorbed,
        into,
        batch,
    }
}
