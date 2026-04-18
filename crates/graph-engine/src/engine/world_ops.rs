//! On-demand world mutations: entity recognition, cohere extraction,
//! relationship decay flush, entity weathering, and change log trim.
//!
//! These are free functions rather than `Engine` methods because they are
//! stateless with respect to the engine (they read no engine config). Keeping
//! them separate from the batch loop also lets `Simulation` call them
//! directly without routing through a method receiver.

use graph_core::{
    BatchId, EmergenceProposal, EndpointKey, Entity, EntityLayer, EntityLineage, EntitySnapshot,
    EntityStatus, EntityWeatheringPolicy, InfluenceKindId, InteractionEffect, LayerTransition,
    LifecycleCause, Relationship, RelationshipId, StateVector, WeatheringEffect, WorldEvent,
    apply_skeleton,
};
use graph_world::World;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::cohere::CoherePerspective;
use crate::emergence::EmergencePerspective;
use crate::engine::batch::{PlasticityObs, TimingOrder};
use crate::registry::{DemotionPolicy, InfluenceKindRegistry};

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

    // Pre-compute resolved extra slots once per distinct kind — avoids
    // O(n_rels) Vec allocations inside the relationship loop.
    let mut slot_cache: FxHashMap<InfluenceKindId, Vec<graph_core::RelationshipSlotDef>> =
        FxHashMap::default();

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
                rel.kind,
                rel.id
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
                let resolved_slots = slot_cache
                    .entry(rel.kind)
                    .or_insert_with(|| influence_registry.resolved_extra_slots(rel.kind));
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
            // Phase 5: auto-pruning removed. Relationships stay around;
            // the caller can explicitly issue a `DeleteRelationship`.
            let _ = cfg;
            None
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
    let coheres = perspective.cluster(world.entities(), world.relationships(), &mut || {
        let id = graph_core::CohereId(counter);
        counter += 1;
        id
    });
    world
        .coheres_mut()
        .update_at(perspective.name(), coheres, batch);
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
    #[cfg(feature = "perf-timing")]
    let t0 = std::time::Instant::now();

    let (_, mut events) = flush_relationship_decay(world, influence_registry);

    #[cfg(feature = "perf-timing")]
    let flush_us = t0.elapsed().as_micros();
    #[cfg(feature = "perf-timing")]
    let t1 = std::time::Instant::now();

    let batch = world.current_batch();
    let proposals = perspective.recognize(world.relationships(), world.entities(), batch);

    #[cfg(feature = "perf-timing")]
    let recognize_us = t1.elapsed().as_micros();
    #[cfg(feature = "perf-timing")]
    let t2 = std::time::Instant::now();

    let proposal_events = apply_proposals(world, proposals, batch);
    events.extend(proposal_events);

    #[cfg(feature = "perf-timing")]
    {
        let apply_us = t2.elapsed().as_micros();
        let rel_count = world.relationships().iter().count();
        eprintln!(
            "[perf-timing] recognize_entities: flush={flush_us}µs recognize={recognize_us}µs apply={apply_us}µs rels={rel_count}"
        );
    }

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

/// Apply per-kind demotion policies after decay, removing qualifying relationships
/// from hot memory.
///
/// Evicted relationships remain in storage (if a backend is configured) and can
/// be promoted back via `Simulation::promote_relationship`. Three policies:
///
/// - `ActivityFloor(f)`: evict when `activity < f`
/// - `IdleBatches(n)`: evict when `current_batch - last_decayed_batch > n`
/// - `LruCapacity(cap)`: keep only the `cap` most recently touched per kind;
///   evict the oldest beyond that cap.
///
/// Returns the IDs of all evicted relationships.
pub(crate) fn apply_demotion_policies(
    world: &mut World,
    influence_registry: &InfluenceKindRegistry,
    current_batch: BatchId,
) -> Vec<RelationshipId> {
    let mut to_evict: FxHashSet<RelationshipId> = FxHashSet::default();

    for kind in influence_registry.kinds() {
        let Some(cfg) = influence_registry.get(kind) else {
            continue;
        };
        let Some(policy) = cfg.demotion_policy else {
            continue;
        };

        match policy {
            DemotionPolicy::ActivityFloor(floor) => {
                for rel in world.relationships().iter() {
                    if rel.kind == kind && rel.activity() < floor {
                        to_evict.insert(rel.id);
                    }
                }
            }
            DemotionPolicy::IdleBatches(n) => {
                for rel in world.relationships().iter() {
                    if rel.kind == kind
                        && current_batch.0.saturating_sub(rel.last_decayed_batch) > n
                    {
                        to_evict.insert(rel.id);
                    }
                }
            }
            DemotionPolicy::LruCapacity(capacity) => {
                let mut rels_of_kind: Vec<(u64, RelationshipId)> = world
                    .relationships()
                    .iter()
                    .filter(|r| r.kind == kind)
                    .map(|r| (r.last_decayed_batch, r.id))
                    .collect();
                if rels_of_kind.len() > capacity {
                    rels_of_kind.sort_unstable_by(|a, b| b.0.cmp(&a.0)); // most-recent first
                    for (_, id) in &rels_of_kind[capacity..] {
                        to_evict.insert(*id);
                    }
                }
            }
        }
    }

    let evicted: Vec<RelationshipId> = to_evict.into_iter().collect();
    for &id in &evicted {
        world.relationships_mut().remove(id);
    }
    evicted
}

/// Apply standard symmetric Hebbian plasticity weight updates.
///
///   `Δweight = η × pre × post`, clamped to `[0, max_weight]`.
///
/// STDP and BCM variants were removed in Phase 3 (audit): the alternative
/// rules had no benchmark evidence of distinct behaviour, so the engine
/// now ships only plain Hebbian.
pub(crate) fn apply_hebbian_updates(
    world: &mut World,
    obs: &[PlasticityObs],
    influence_registry: &InfluenceKindRegistry,
) -> Vec<(RelationshipId, InfluenceKindId, StateVector, StateVector)> {
    let mut changed: Vec<(RelationshipId, InfluenceKindId, StateVector, StateVector)> = Vec::new();

    for &PlasticityObs {
        rel_id,
        kind,
        pre,
        post,
        timing: _,
        post_locus: _,
    } in obs
    {
        let Some(cfg) = influence_registry.get(kind) else {
            continue;
        };
        let eta = cfg.plasticity.learning_rate;
        let max_w = cfg.plasticity.max_weight;

        if let Some(rel) = world.relationships_mut().get_mut(rel_id) {
            let cur_w = rel
                .state
                .as_slice()
                .get(Relationship::WEIGHT_SLOT)
                .copied()
                .unwrap_or(0.0);
            let new_w = (cur_w + eta * pre * post).clamp(0.0, max_w);
            if (new_w - cur_w).abs() > 1e-9 {
                let before = rel.state.clone();
                rel.state.as_mut_slice()[Relationship::WEIGHT_SLOT] = new_w;
                changed.push((rel_id, kind, before, rel.state.clone()));
            }
        }
    }

    changed
}

/// Apply cross-kind interaction effects for endpoint pairs touched by 2+
/// distinct influence kinds in a single batch.
///
/// Pairs of kinds are enumerated; if a registered `InteractionEffect` exists
/// for the pair, its multiplier is accumulated multiplicatively. The composed
/// factor is then applied once to the activity slot of every relationship
/// between those endpoints, making the result order-independent.
pub(crate) fn apply_interaction_effects(
    world: &mut World,
    batch_kind_touches: &FxHashMap<
        EndpointKey,
        (FxHashSet<InfluenceKindId>, FxHashSet<RelationshipId>),
    >,
    influence_registry: &InfluenceKindRegistry,
) {
    for (_ep_key, (touched_kinds, rel_ids)) in batch_kind_touches {
        if touched_kinds.len() < 2 {
            continue;
        }
        let kinds: Vec<InfluenceKindId> = touched_kinds.iter().copied().collect();
        let mut multiplier = 1.0f32;
        for i in 0..kinds.len() {
            for j in (i + 1)..kinds.len() {
                if let Some(effect) = influence_registry.interaction_between(kinds[i], kinds[j]) {
                    multiplier *= match effect {
                        InteractionEffect::Synergistic { boost } => *boost,
                        InteractionEffect::Antagonistic { dampen } => *dampen,
                        InteractionEffect::Neutral => 1.0,
                    };
                }
            }
        }
        if (multiplier - 1.0).abs() > f32::EPSILON {
            for rel_id in rel_ids {
                if let Some(rel) = world.relationships_mut().get_mut(*rel_id) {
                    if let Some(a) = rel
                        .state
                        .as_mut_slice()
                        .get_mut(Relationship::ACTIVITY_SLOT)
                    {
                        *a *= multiplier;
                    }
                }
            }
        }
    }
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
            EmergenceProposal::Born {
                members,
                member_relationships,
                coherence,
                parents,
                cause,
            } => {
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
                entity.lineage = EntityLineage {
                    parents,
                    children: Vec::new(),
                };
                store.insert(entity);
                events.push(WorldEvent::EntityBorn {
                    entity: id,
                    batch,
                    member_count,
                });
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
                        EntityLayer::new(batch, e.current.clone(), LayerTransition::BecameDormant)
                            .with_cause(cause),
                    );
                    events.push(WorldEvent::EntityDormant { entity, batch });
                }
            }
            EmergenceProposal::Revive {
                entity,
                snapshot,
                cause,
            } => {
                if let Some(e) = world.entities_mut().get_mut(entity) {
                    e.status = EntityStatus::Active;
                    let layer = EntityLayer::new(batch, snapshot.clone(), LayerTransition::Revived)
                        .with_cause(cause);
                    e.current = snapshot;
                    e.layers.push(layer);
                    events.push(WorldEvent::EntityRevived { entity, batch });
                }
            }
            EmergenceProposal::Split {
                source,
                offspring,
                cause,
            } => {
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
                    events.push(WorldEvent::EntityBorn {
                        entity: child_id,
                        batch,
                        member_count,
                    });
                }
                if let Some(e) = world.entities_mut().get_mut(source) {
                    let layer = EntityLayer::new(
                        batch,
                        e.current.clone(),
                        LayerTransition::Split {
                            offspring: child_ids.clone(),
                        },
                    )
                    .with_cause(cause);
                    e.layers.push(layer);
                    e.lineage.children.extend(child_ids.clone());
                    // Source ceases to be an independently recognized
                    // community once its offspring take over — without this,
                    // the next `recognize_entities` pass would re-match the
                    // offspring components against the source's unchanged
                    // `current.members` and spuriously fold them back in as
                    // `Merged`. Post-split the source is semantically dormant
                    // (its identity continues through `lineage.children`),
                    // not absorbed — so no extra `BecameDormant` layer: the
                    // `Split` layer already marks the lifecycle transition.
                    e.status = EntityStatus::Dormant;
                }
                events.push(WorldEvent::EntitySplit {
                    source,
                    offspring: child_ids,
                    batch,
                });
            }
            EmergenceProposal::Merge {
                absorbed,
                into,
                new_members,
                member_relationships,
                coherence,
                cause,
            } => {
                for absorbed_id in &absorbed {
                    if let Some(e) = world.entities_mut().get_mut(*absorbed_id) {
                        e.status = EntityStatus::Dormant;
                        e.layers.push(
                            EntityLayer::new(
                                batch,
                                e.current.clone(),
                                LayerTransition::Merged {
                                    absorbed: vec![into],
                                },
                            )
                            .with_cause(LifecycleCause::MergedInto { survivor: into }),
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
                        LayerTransition::Merged {
                            absorbed: absorbed.clone(),
                        },
                    )
                    .with_cause(cause);
                    e.current = snapshot;
                    e.layers.push(layer);
                    e.lineage.children.extend(absorbed.clone());
                }
                events.push(WorldEvent::EntityMerged {
                    absorbed,
                    into,
                    batch,
                });
            }
        }
    }

    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{
        BatchId, Endpoints, InfluenceKindId, Locus, LocusId, LocusKindId, Relationship,
        RelationshipId, RelationshipLineage, StateVector,
    };
    use graph_world::World;
    use smallvec::SmallVec;

    use crate::engine::batch::{PlasticityObs, TimingOrder};
    use crate::registry::{
        DemotionPolicy, InfluenceKindConfig, InfluenceKindRegistry, PlasticityConfig,
    };

    fn make_world_with_rel(activity: f32, weight: f32) -> (World, RelationshipId) {
        let mut world = World::default();
        world.loci_mut().insert(Locus::new(
            LocusId(1),
            LocusKindId(0),
            StateVector::zeros(1),
        ));
        world.loci_mut().insert(Locus::new(
            LocusId(2),
            LocusKindId(0),
            StateVector::zeros(1),
        ));
        let rel = Relationship {
            id: RelationshipId(0),
            kind: InfluenceKindId(1),
            endpoints: Endpoints::symmetric(LocusId(1), LocusId(2)),
            state: StateVector::from_slice(&[activity, weight]),
            lineage: RelationshipLineage {
                created_by: None,
                last_touched_by: None,
                change_count: 0,
                kinds_observed: SmallVec::new(),
            },
            created_batch: BatchId(0),
            last_decayed_batch: 0,
            metadata: None,
        };
        let rel_id = rel.id;
        world.relationships_mut().insert(rel);
        (world, rel_id)
    }

    fn make_rel(
        world: &mut World,
        id: u64,
        kind: u64,
        activity: f32,
        last_decayed_batch: u64,
    ) -> RelationshipId {
        let rel_id = RelationshipId(id);
        world.loci_mut().insert(Locus::new(
            LocusId(id * 2),
            LocusKindId(0),
            StateVector::zeros(1),
        ));
        world.loci_mut().insert(Locus::new(
            LocusId(id * 2 + 1),
            LocusKindId(0),
            StateVector::zeros(1),
        ));
        let mut rel = Relationship {
            id: rel_id,
            kind: InfluenceKindId(kind),
            endpoints: Endpoints::symmetric(LocusId(id * 2), LocusId(id * 2 + 1)),
            state: StateVector::from_slice(&[activity, 0.0]),
            lineage: RelationshipLineage {
                created_by: None,
                last_touched_by: None,
                change_count: 0,
                kinds_observed: SmallVec::new(),
            },
            created_batch: BatchId(0),
            last_decayed_batch: 0,
            metadata: None,
        };
        rel.last_decayed_batch = last_decayed_batch;
        world.relationships_mut().insert(rel);
        rel_id
    }

    #[test]
    fn demotion_activity_floor_evicts_low_activity() {
        let mut world = World::default();
        let low = make_rel(&mut world, 10, 1, 0.05, 0);
        let high = make_rel(&mut world, 11, 1, 0.9, 0);

        let mut reg = InfluenceKindRegistry::new();
        reg.insert(
            InfluenceKindId(1),
            InfluenceKindConfig::new("t").with_demotion(DemotionPolicy::ActivityFloor(0.1)),
        );

        apply_demotion_policies(&mut world, &reg, BatchId(5));

        assert!(
            world.relationships().get(low).is_none(),
            "low activity should be evicted"
        );
        assert!(
            world.relationships().get(high).is_some(),
            "high activity should remain"
        );
    }

    #[test]
    fn demotion_idle_batches_evicts_stale_rels() {
        let mut world = World::default();
        let stale = make_rel(&mut world, 20, 2, 1.0, 0); // last touched at batch 0
        let fresh = make_rel(&mut world, 21, 2, 1.0, 9); // last touched at batch 9

        let mut reg = InfluenceKindRegistry::new();
        reg.insert(
            InfluenceKindId(2),
            InfluenceKindConfig::new("t").with_demotion(DemotionPolicy::IdleBatches(5)),
        );

        // current_batch=10: stale idle 10 > 5 → evict; fresh idle 1 ≤ 5 → keep
        apply_demotion_policies(&mut world, &reg, BatchId(10));

        assert!(
            world.relationships().get(stale).is_none(),
            "stale should be evicted"
        );
        assert!(
            world.relationships().get(fresh).is_some(),
            "fresh should remain"
        );
    }

    #[test]
    fn demotion_lru_capacity_keeps_most_recent() {
        let mut world = World::default();
        // Three rels of kind 3, touched at batches 1, 5, 9 respectively.
        let oldest = make_rel(&mut world, 30, 3, 1.0, 1);
        let middle = make_rel(&mut world, 31, 3, 1.0, 5);
        let newest = make_rel(&mut world, 32, 3, 1.0, 9);

        let mut reg = InfluenceKindRegistry::new();
        reg.insert(
            InfluenceKindId(3),
            InfluenceKindConfig::new("t").with_demotion(DemotionPolicy::LruCapacity(2)),
        );

        apply_demotion_policies(&mut world, &reg, BatchId(10));

        assert!(
            world.relationships().get(oldest).is_none(),
            "oldest should be evicted beyond cap=2"
        );
        assert!(
            world.relationships().get(middle).is_some(),
            "middle (2nd recent) should remain"
        );
        assert!(
            world.relationships().get(newest).is_some(),
            "newest should remain"
        );
    }

    #[test]
    fn demotion_lru_below_capacity_evicts_nothing() {
        let mut world = World::default();
        let a = make_rel(&mut world, 40, 4, 1.0, 5);
        let b = make_rel(&mut world, 41, 4, 1.0, 8);

        let mut reg = InfluenceKindRegistry::new();
        reg.insert(
            InfluenceKindId(4),
            InfluenceKindConfig::new("t").with_demotion(DemotionPolicy::LruCapacity(5)),
        );

        apply_demotion_policies(&mut world, &reg, BatchId(10));

        assert!(world.relationships().get(a).is_some());
        assert!(world.relationships().get(b).is_some());
    }

    #[test]
    fn hebbian_delta_weight_formula() {
        let (mut world, rel_id) = make_world_with_rel(1.0, 0.0);
        let mut reg = InfluenceKindRegistry::new();
        reg.insert(
            InfluenceKindId(1),
            InfluenceKindConfig::new("t").with_plasticity(PlasticityConfig {
                learning_rate: 0.1,
                weight_decay: 1.0,
                max_weight: 10.0,
                ..Default::default()
            }),
        );

        // Δw = 0.1 × 0.8 × 0.9 = 0.072
        let _ = apply_hebbian_updates(
            &mut world,
            &[PlasticityObs {
                rel_id,
                kind: InfluenceKindId(1),
                pre: 0.8,
                post: 0.9,
                timing: TimingOrder::Simultaneous,
                post_locus: LocusId(2),
            }],
            &reg,
        );

        let w = world.relationships().get(rel_id).unwrap().weight();
        assert!((w - 0.072).abs() < 1e-6, "weight = {w}");
    }

    #[test]
    fn hebbian_clamps_at_max_weight() {
        let (mut world, rel_id) = make_world_with_rel(1.0, 9.9);
        let mut reg = InfluenceKindRegistry::new();
        reg.insert(
            InfluenceKindId(1),
            InfluenceKindConfig::new("t").with_plasticity(PlasticityConfig {
                learning_rate: 1.0,
                weight_decay: 1.0,
                max_weight: 10.0,
                ..Default::default()
            }),
        );

        apply_hebbian_updates(
            &mut world,
            &[PlasticityObs {
                rel_id,
                kind: InfluenceKindId(1),
                pre: 1.0,
                post: 1.0,
                timing: TimingOrder::Simultaneous,
                post_locus: LocusId(2),
            }],
            &reg,
        );

        let w = world.relationships().get(rel_id).unwrap().weight();
        assert!((w - 10.0).abs() < 1e-6, "weight = {w}");
    }

    #[test]
    fn hebbian_no_op_when_learning_rate_zero() {
        let (mut world, rel_id) = make_world_with_rel(1.0, 3.0);
        let mut reg = InfluenceKindRegistry::new();
        reg.insert(InfluenceKindId(1), InfluenceKindConfig::new("t"));

        apply_hebbian_updates(
            &mut world,
            &[PlasticityObs {
                rel_id,
                kind: InfluenceKindId(1),
                pre: 1.0,
                post: 1.0,
                timing: TimingOrder::Simultaneous,
                post_locus: LocusId(2),
            }],
            &reg,
        );

        let w = world.relationships().get(rel_id).unwrap().weight();
        assert!(
            (w - 3.0).abs() < 1e-6,
            "weight should be unchanged, got {w}"
        );
    }

    fn make_stdp_reg() -> InfluenceKindRegistry {
        let mut reg = InfluenceKindRegistry::new();
        reg.insert(
            InfluenceKindId(1),
            InfluenceKindConfig::new("stdp").with_plasticity(PlasticityConfig {
                learning_rate: 0.1,
                weight_decay: 1.0,
                max_weight: 10.0,

                ..Default::default()
            }),
        );
        reg
    }

    #[test]
    fn stdp_causal_strengthens() {
        let (mut world, rel_id) = make_world_with_rel(1.0, 0.0);
        let reg = make_stdp_reg();
        apply_hebbian_updates(
            &mut world,
            &[PlasticityObs {
                rel_id,
                kind: InfluenceKindId(1),
                pre: 0.8,
                post: 0.9,
                timing: TimingOrder::PreFirst,
                post_locus: LocusId(2),
            }],
            &reg,
        );
        let w = world.relationships().get(rel_id).unwrap().weight();
        assert!(w > 0.0, "causal STDP should increase weight, got {w}");
        assert!((w - 0.072).abs() < 1e-6, "expected 0.072, got {w}");
    }

    // `stdp_anticausal_*` tests removed with Phase 3 — timing-dependent
    // asymmetry is no longer engine behaviour.

    #[test]
    fn stdp_simultaneous_strengthens() {
        let (mut world, rel_id) = make_world_with_rel(1.0, 0.0);
        let reg = make_stdp_reg();
        apply_hebbian_updates(
            &mut world,
            &[PlasticityObs {
                rel_id,
                kind: InfluenceKindId(1),
                pre: 0.8,
                post: 0.9,
                timing: TimingOrder::Simultaneous,
                post_locus: LocusId(2),
            }],
            &reg,
        );
        let w = world.relationships().get(rel_id).unwrap().weight();
        assert!(w > 0.0, "simultaneous STDP should increase weight, got {w}");
        assert!((w - 0.072).abs() < 1e-6, "expected 0.072, got {w}");
    }

    // BCM LTP/LTD unit tests removed with Phase 3 — BCM rule no longer
    // part of the engine.
}
