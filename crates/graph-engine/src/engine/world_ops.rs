//! On-demand world mutations: entity recognition, cohere extraction,
//! relationship decay flush, entity weathering, and change log trim.
//!
//! These are free functions rather than `Engine` methods because they are
//! stateless with respect to the engine (they read no engine config). Keeping
//! them separate from the batch loop also lets `Simulation` call them
//! directly without routing through a method receiver.

use graph_core::{
    apply_skeleton, BatchId, EmergenceProposal, Entity, EntityLayer, EntityLineage,
    EntitySnapshot, EntityStatus, EntityWeatheringPolicy, LayerTransition,
    LifecycleCause, Relationship, WeatheringEffect, WorldEvent,
};
use graph_world::World;

use crate::cohere::CoherePerspective;
use crate::emergence::EmergencePerspective;
use crate::registry::InfluenceKindRegistry;

/// Flush all pending lazy decay for every relationship.
///
/// The engine uses lazy decay: relationship activity/weight slots are
/// only updated when the relationship is touched (auto-emerge) or when
/// this function is called. Call this before reading relationship activity
/// values (e.g. before `recognize_entities` or `extract_cohere`).
///
/// Flush all pending lazy decay and auto-prune dead relationships.
///
/// Returns `(pruned_count, events)` — the number of relationships pruned
/// and a `WorldEvent::RelationshipPruned` for each one.
pub(crate) fn flush_relationship_decay(
    world: &mut World,
    influence_registry: &InfluenceKindRegistry,
) -> (usize, Vec<WorldEvent>) {
    let current_batch = world.current_batch().0;

    let to_prune: Vec<graph_core::RelationshipId> = world
        .relationships_mut()
        .iter_mut()
        .filter_map(|rel| {
            let delta = current_batch.saturating_sub(rel.last_decayed_batch);
            let cfg = influence_registry.get(rel.kind);
            debug_assert!(
                cfg.is_some(),
                "flush_relationship_decay: InfluenceKindId {:?} is not registered — \
                 relationship {:?} will not be decayed or pruned. \
                 Register it with InfluenceKindRegistry::insert().",
                rel.kind, rel.id
            );
            if delta > 0 {
                let (act_decay, wt_decay) = cfg
                    .map(|c| (c.decay_per_batch, c.plasticity.weight_decay))
                    .unwrap_or((1.0, 1.0));
                let act_factor = act_decay.powi(delta as i32);
                let wt_factor = wt_decay.powi(delta as i32);
                let slots = rel.state.as_mut_slice();
                if let Some(a) = slots.get_mut(Relationship::ACTIVITY_SLOT) {
                    *a *= act_factor;
                }
                if let Some(w) = slots.get_mut(Relationship::WEIGHT_SLOT) {
                    *w *= wt_factor;
                }
                // Decay extra slots with their per-slot rates, including
                // inherited slots from ancestor kinds.
                let resolved_slots = influence_registry.resolved_extra_slots(rel.kind);
                for (i, slot_def) in resolved_slots.iter().enumerate() {
                    if let Some(factor) = slot_def.decay {
                        let idx = 2 + i;
                        if let Some(v) = slots.get_mut(idx) {
                            *v *= factor.powi(delta as i32);
                        }
                    }
                }
                rel.last_decayed_batch = current_batch;
            }
            // Check if this relationship should be pruned.
            let threshold = cfg.map(|c| c.prune_activity_threshold).unwrap_or(0.0);
            if threshold > 0.0 && rel.activity() < threshold {
                Some(rel.id)
            } else {
                None
            }
        })
        .collect();

    // Phase 2: remove pruned relationships (requires &mut, sequential).
    // Also clean up any subscriptions pointing at pruned relationships so
    // subscribers don't receive notifications for non-existent edges.
    // Note: tombstone notifications for decay-pruned relationships are not
    // emitted here because flush_relationship_decay is called on-demand
    // (outside the batch loop) and has no access to the pending queue.
    // Decay-pruned edges have near-zero activity, so subscriber programs
    // are typically idle and the omission is acceptable.
    let pruned = to_prune.len();
    let events: Vec<WorldEvent> = to_prune
        .iter()
        .map(|&id| WorldEvent::RelationshipPruned { relationship: id })
        .collect();
    for rel_id in to_prune {
        world.subscriptions_mut().remove_relationship(rel_id);
        world.relationships_mut().remove(rel_id);
        world.record_pruned(rel_id);
    }
    (pruned, events)
}

/// Run a `CoherePerspective` and store the resulting clusters.
///
/// Flushes pending relationship decay before clustering so that
/// activity values reflect the true current state.
///
/// Replaces the previous cohere set for this perspective's name in the
/// `CohereStore`. Per `docs/redesign.md` §6 step 8: "Optional, on-demand."
pub(crate) fn extract_cohere(
    world: &mut World,
    influence_registry: &InfluenceKindRegistry,
    perspective: &dyn CoherePerspective,
) {
    let _ = flush_relationship_decay(world, influence_registry);
    let batch = world.current_batch();
    let store = world.coheres_mut();
    let mut counter = store.mint_id().0;
    let coheres = perspective.cluster(
        world.entities(),
        world.relationships(),
        &mut || {
            let id = graph_core::CohereId(counter);
            counter += 1;
            id
        },
    );
    world.coheres_mut().update_at(perspective.name(), coheres, batch);
}

/// Apply an `EmergencePerspective` to the current world state and
/// commit its proposals to the entity store.
///
/// Flushes pending relationship decay before recognition so that
/// activity values reflect the true current state.
///
/// This is on-demand — the caller decides when entity recognition
/// should happen. Per `docs/redesign.md` §6 step 7: "Optional, on-demand."
pub(crate) fn recognize_entities(
    world: &mut World,
    influence_registry: &InfluenceKindRegistry,
    perspective: &dyn EmergencePerspective,
) -> Vec<WorldEvent> {
    let (_, mut events) = flush_relationship_decay(world, influence_registry);
    let batch = world.current_batch();
    let proposals = perspective.recognize(world.relationships(), world.entities(), batch);
    let proposal_events = apply_proposals(world, proposals, batch);
    events.extend(proposal_events);
    events
}

/// Apply a weathering policy to every entity's sediment layer stack.
///
/// On-demand — call every N ticks rather than after every tick.
pub(crate) fn weather_entities(world: &mut World, policy: &dyn EntityWeatheringPolicy) {
    let current_batch = world.current_batch().0;
    for entity in world.entities_mut().iter_mut() {
        let mut i = 0;
        while i < entity.layers.len() {
            let age = current_batch.saturating_sub(entity.layers[i].batch.0);
            let effect = policy.effect(&entity.layers[i], age);
            match effect {
                WeatheringEffect::Preserved => {
                    i += 1;
                }
                WeatheringEffect::Compress => {
                    graph_core::apply_compress(&mut entity.layers[i]);
                    i += 1;
                }
                WeatheringEffect::Skeleton => {
                    apply_skeleton(&mut entity.layers[i]);
                    i += 1;
                }
                WeatheringEffect::Remove => {
                    if entity.layers[i].transition.is_significant() {
                        // Never delete Born/Split/Merged — skeleton instead.
                        apply_skeleton(&mut entity.layers[i]);
                        i += 1;
                    } else {
                        entity.layers.remove(i);
                        // i stays the same — now points to the next layer.
                    }
                }
            }
        }
    }
}

/// Trim the change log, dropping all changes in batches strictly older
/// than `current_batch - retention_batches`. Returns the number of
/// change records removed.
pub(crate) fn trim_change_log(world: &mut World, retention_batches: u64) -> usize {
    let current = world.current_batch().0;
    let retain_from = graph_core::BatchId(current.saturating_sub(retention_batches));
    world.log_mut().trim_before_batch(retain_from)
}

/// Apply a list of emergence proposals to the entity store, returning
/// a `WorldEvent` for each lifecycle transition.
pub(crate) fn apply_proposals(
    world: &mut World,
    proposals: Vec<EmergenceProposal>,
    batch: BatchId,
) -> Vec<WorldEvent> {
    let mut events = Vec::new();

    for proposal in proposals {
        match proposal {
            EmergenceProposal::Born { members, member_relationships, coherence, parents, cause } => {
                let member_count = members.len();
                let snapshot = EntitySnapshot {
                    members,
                    member_relationships,
                    coherence,
                };
                let store = world.entities_mut();
                let id = store.mint_id();
                let mut entity = Entity::born(id, batch, snapshot);
                // Attach cause to the birth layer.
                if let Some(layer) = entity.layers.last_mut() {
                    layer.cause = cause;
                }
                entity.lineage = EntityLineage { parents, children: Vec::new() };
                store.insert(entity);
                events.push(WorldEvent::EntityBorn { entity: id, batch, member_count });
            }
            EmergenceProposal::DepositLayer { entity, layer } => {
                if let Some(e) = world.entities_mut().get_mut(entity) {
                    if let LayerTransition::CoherenceShift { from, to } = &layer.transition {
                        events.push(WorldEvent::CoherenceShift {
                            entity,
                            from: *from,
                            to: *to,
                            batch,
                        });
                    }
                    e.current = layer.snapshot.clone().unwrap_or_default();
                    e.layers.push(layer);
                }
            }
            EmergenceProposal::Dormant { entity, cause } => {
                if let Some(e) = world.entities_mut().get_mut(entity) {
                    e.status = EntityStatus::Dormant;
                    e.layers.push(
                        EntityLayer::new(
                            batch,
                            e.current.clone(),
                            LayerTransition::BecameDormant,
                        )
                        .with_cause(cause),
                    );
                    events.push(WorldEvent::EntityDormant { entity, batch });
                }
            }
            EmergenceProposal::Revive { entity, snapshot, cause } => {
                if let Some(e) = world.entities_mut().get_mut(entity) {
                    e.status = EntityStatus::Active;
                    let layer = EntityLayer::new(batch, snapshot.clone(), LayerTransition::Revived)
                        .with_cause(cause);
                    e.current = snapshot;
                    e.layers.push(layer);
                    events.push(WorldEvent::EntityRevived { entity, batch });
                }
            }
            EmergenceProposal::Split { source, offspring, cause } => {
                let mut child_ids = Vec::new();
                for (members, member_relationships, coherence) in offspring {
                    let member_count = members.len();
                    let snapshot = EntitySnapshot {
                        members,
                        member_relationships,
                        coherence,
                    };
                    let store = world.entities_mut();
                    let child_id = store.mint_id();
                    let child = Entity::born(child_id, batch, snapshot);
                    store.insert(child);
                    child_ids.push(child_id);
                    events.push(WorldEvent::EntityBorn { entity: child_id, batch, member_count });
                }
                if let Some(e) = world.entities_mut().get_mut(source) {
                    let layer = EntityLayer::new(
                        batch,
                        e.current.clone(),
                        LayerTransition::Split { offspring: child_ids.clone() },
                    )
                    .with_cause(cause);
                    e.layers.push(layer);
                    e.lineage.children.extend(child_ids.clone());
                }
                events.push(WorldEvent::EntitySplit {
                    source,
                    offspring: child_ids,
                    batch,
                });
            }
            EmergenceProposal::Merge { absorbed, into, new_members, member_relationships, coherence, cause } => {
                for absorbed_id in &absorbed {
                    if let Some(e) = world.entities_mut().get_mut(*absorbed_id) {
                        e.status = EntityStatus::Dormant;
                        e.layers.push(
                            EntityLayer::new(
                                batch,
                                e.current.clone(),
                                LayerTransition::Merged { absorbed: vec![into] },
                            )
                            .with_cause(LifecycleCause::MergedInto {
                                survivor: into,
                            }),
                        );
                    }
                }
                let snapshot = EntitySnapshot {
                    members: new_members,
                    member_relationships,
                    coherence,
                };
                if let Some(e) = world.entities_mut().get_mut(into) {
                    let layer = EntityLayer::new(
                        batch,
                        snapshot.clone(),
                        LayerTransition::Merged { absorbed: absorbed.clone() },
                    )
                    .with_cause(cause);
                    e.current = snapshot;
                    e.layers.push(layer);
                    e.lineage.children.extend(absorbed.clone());
                }
                events.push(WorldEvent::EntityMerged { absorbed, into, batch });
            }
        }
    }

    events
}
