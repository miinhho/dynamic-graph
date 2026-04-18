//! Entity-level causality tracing (D4).
//!
//! Given an entity and the batch at which a lifecycle transition occurred,
//! this module traces *why* that transition happened — walking from the
//! `LifecycleCause` (which names key relationships) through `causal_ancestors`
//! in the `ChangeLog` to find upstream entity transitions that caused them.
//!
//! ## Typical usage
//!
//! ```ignore
//! let batch = BatchId(42);
//! let causes = graph_query::entity_upstream_transitions(&world, entity_id, batch);
//! for (upstream_id, upstream_batch) in causes {
//!     println!("Entity {:?} had a transition at batch {:?}", upstream_id, upstream_batch);
//! }
//! ```
//!
//! ## Approach
//!
//! 1. Look up the `EntityLayer` for `(entity_id, batch)`.
//! 2. Extract `RelationshipId`s from the `LifecycleCause` (key relationships).
//! 3. Get the most recent `ChangeId` for each key relationship (via
//!    `changes_to_relationship`).
//! 4. Run `causal_ancestors` over those seed change ids.
//! 5. For each ancestor change that targets a locus, check whether any
//!    entity's most recent layer before `batch` lists that locus as a member.
//! 6. Return the deduplicated set of `(EntityId, BatchId)` pairs.

use std::collections::HashSet;

use graph_core::{BatchId, EntityId, LocusId, RelationshipId};
use graph_world::World;

use crate::causality::causal_ancestors;

// ── Public API ────────────────────────────────────────────────────────────────

/// Return the `LifecycleCause` for `entity_id` at batch `at_batch`, if any.
///
/// Looks for the entity layer deposited at exactly `at_batch`. Returns `None`
/// when the entity does not exist or has no layer at that batch.
pub fn entity_transition_cause(
    world: &World,
    entity_id: EntityId,
    at_batch: BatchId,
) -> Option<graph_core::LifecycleCause> {
    let layer = world.entities().layer_at_batch(entity_id, at_batch)?;
    Some(layer.cause.clone())
}

/// Return the change ids for the key relationships named in a lifecycle cause.
///
/// For `RelationshipDecay`, `ComponentSplit`, and `RelationshipCluster` causes
/// this returns the most recent change for each named relationship up to
/// (but not including) `before_batch`. This bounds the ancestor search to
/// the causal history that predates the transition.
///
/// Returns an empty `Vec` for `Unspecified`, `MergedFrom`, and `MergedInto`
/// causes (those have no associated relationship changes to trace).
pub fn cause_seed_changes(
    world: &World,
    cause: &graph_core::LifecycleCause,
    before_batch: BatchId,
) -> Vec<graph_core::ChangeId> {
    let rel_ids: &[RelationshipId] = match cause {
        graph_core::LifecycleCause::RelationshipCluster { key_relationships } => key_relationships,
        graph_core::LifecycleCause::RelationshipDecay {
            decayed_relationships,
        } => decayed_relationships,
        graph_core::LifecycleCause::ComponentSplit { weak_bridges } => weak_bridges,
        _ => return Vec::new(),
    };

    let mut seeds = Vec::new();
    for &rel_id in rel_ids {
        // Find the most recent change to this relationship before `before_batch`.
        if let Some(change_id) = world
            .log()
            .changes_to_relationship(rel_id)
            .filter(|c| c.batch < before_batch)
            .map(|c| c.id)
            .next()
        {
            seeds.push(change_id);
        }
    }
    seeds
}

/// Find upstream entity transitions that are causally responsible for the
/// transition of `entity_id` at batch `at_batch`.
///
/// The algorithm:
/// 1. Get the `LifecycleCause` for the entity at `at_batch`.
/// 2. Collect seed change ids from the cause's key relationships.
/// 3. Expand via `causal_ancestors`.
/// 4. For each ancestor change targeting a locus, find entities whose member
///    list included that locus at a layer deposited before `at_batch`.
///
/// Returns a deduplicated `Vec<(EntityId, BatchId)>` where `BatchId` is the
/// batch of the most recent layer of that entity *before* `at_batch`.
pub fn entity_upstream_transitions(
    world: &World,
    entity_id: EntityId,
    at_batch: BatchId,
) -> Vec<(EntityId, BatchId)> {
    let cause = match entity_transition_cause(world, entity_id, at_batch) {
        Some(c) => c,
        None => return Vec::new(),
    };

    let seeds = cause_seed_changes(world, &cause, at_batch);
    if seeds.is_empty() {
        return Vec::new();
    }

    // Collect all ancestor change ids (includes seeds themselves).
    let mut all_ancestors: HashSet<graph_core::ChangeId> = HashSet::new();
    for seed in seeds {
        all_ancestors.insert(seed);
        for anc in causal_ancestors(world, seed) {
            all_ancestors.insert(anc);
        }
    }

    // For each ancestor change, extract the locus subject (skip relationship changes).
    let ancestor_loci: HashSet<LocusId> = all_ancestors
        .iter()
        .filter_map(|&cid| world.log().get(cid))
        .filter_map(|c| {
            if let graph_core::ChangeSubject::Locus(lid) = c.subject {
                Some(lid)
            } else {
                None
            }
        })
        .collect();

    if ancestor_loci.is_empty() {
        return Vec::new();
    }

    // Find entities (excluding the query entity) that had at least one of
    // these loci as a member at some layer before `at_batch`.
    let mut result: Vec<(EntityId, BatchId)> = Vec::new();
    let mut seen_entities: HashSet<EntityId> = HashSet::new();
    seen_entities.insert(entity_id);

    'outer: for entity in world.entities().iter() {
        if seen_entities.contains(&entity.id) {
            continue;
        }
        // Walk layers newest-first to find the most recent layer before `at_batch`.
        let mut best_batch: Option<BatchId> = None;
        for layer in entity.layers.iter().rev() {
            if layer.batch >= at_batch {
                continue;
            }
            // Check if this layer's snapshot includes any ancestor locus.
            if let Some(snapshot) = &layer.snapshot {
                let overlap = snapshot.members.iter().any(|m| ancestor_loci.contains(m));
                if overlap {
                    best_batch = Some(layer.batch);
                    break;
                }
            }
        }
        if let Some(b) = best_batch {
            // Also verify this entity had a transition at that batch (i.e.,
            // there is a layer deposited at `b`).
            if world.entities().layer_at_batch(entity.id, b).is_some() {
                seen_entities.insert(entity.id);
                result.push((entity.id, b));
                if result.len() >= 256 {
                    break 'outer;
                }
            }
        }
    }

    result
}

/// Return all entity lifecycle layers for `entity_id` that fall within
/// the batch range `[from, to)`.
///
/// Layers are returned oldest-first, matching the entity's sediment stack order.
pub fn entity_layers_in_range(
    world: &World,
    entity_id: EntityId,
    from: BatchId,
    to: BatchId,
) -> Vec<(
    BatchId,
    graph_core::LayerTransition,
    graph_core::LifecycleCause,
)> {
    let entity = match world.entities().get(entity_id) {
        Some(e) => e,
        None => return Vec::new(),
    };
    entity
        .layers
        .iter()
        .filter(|l| l.batch >= from && l.batch < to)
        .map(|l| (l.batch, l.transition.clone(), l.cause.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{
        BatchId, Change, ChangeId, ChangeSubject, EntityId, EntitySnapshot, EntityStatus,
        InfluenceKindId, LayerTransition, LifecycleCause, LocusId, RelationshipId, StateVector,
    };
    use graph_world::World;

    fn empty_world() -> World {
        World::new()
    }

    fn make_entity(id: EntityId, batch: BatchId, members: Vec<LocusId>) -> graph_core::Entity {
        let snapshot = EntitySnapshot {
            members,
            member_relationships: Vec::new(),
            coherence: 0.8,
        };
        graph_core::Entity::born(id, batch, snapshot)
    }

    #[test]
    fn entity_transition_cause_missing_returns_none() {
        let world = empty_world();
        let result = entity_transition_cause(&world, EntityId(1), BatchId(0));
        assert!(result.is_none());
    }

    #[test]
    fn entity_layers_in_range_empty_entity_returns_empty() {
        let world = empty_world();
        let result = entity_layers_in_range(&world, EntityId(99), BatchId(0), BatchId(100));
        assert!(result.is_empty());
    }

    #[test]
    fn entity_upstream_transitions_no_cause_returns_empty() {
        let mut world = empty_world();
        let eid = EntityId(1);
        let batch = BatchId(5);
        let e = make_entity(eid, batch, vec![LocusId(0)]);
        world.entities_mut().insert(e);
        // Born layer has LifecycleCause::Unspecified, so seeds are empty.
        let result = entity_upstream_transitions(&world, eid, batch);
        assert!(result.is_empty());
    }

    #[test]
    fn cause_seed_changes_unspecified_returns_empty() {
        let world = empty_world();
        let seeds = cause_seed_changes(&world, &LifecycleCause::Unspecified, BatchId(10));
        assert!(seeds.is_empty());
    }

    #[test]
    fn cause_seed_changes_merged_from_returns_empty() {
        let world = empty_world();
        let seeds = cause_seed_changes(
            &world,
            &LifecycleCause::MergedFrom {
                absorbed: vec![EntityId(2)],
            },
            BatchId(10),
        );
        assert!(seeds.is_empty());
    }

    #[test]
    fn entity_layers_in_range_filters_correctly() {
        let mut world = empty_world();
        let eid = EntityId(1);

        // Born at batch 2.
        let e = make_entity(eid, BatchId(2), vec![LocusId(0)]);
        world.entities_mut().insert(e);

        // Deposit another layer at batch 5.
        {
            let e = world.entities_mut().get_mut(eid).unwrap();
            let snap = EntitySnapshot {
                members: vec![LocusId(0), LocusId(1)],
                member_relationships: vec![],
                coherence: 0.9,
            };
            e.deposit(
                BatchId(5),
                snap,
                LayerTransition::MembershipDelta {
                    added: vec![LocusId(1)],
                    removed: vec![],
                },
            );
        }

        // Query [0, 4) → should include birth layer only.
        let r1 = entity_layers_in_range(&world, eid, BatchId(0), BatchId(4));
        assert_eq!(r1.len(), 1);
        assert_eq!(r1[0].0, BatchId(2));

        // Query [3, 10) → should include the layer at batch 5.
        let r2 = entity_layers_in_range(&world, eid, BatchId(3), BatchId(10));
        assert_eq!(r2.len(), 1);
        assert_eq!(r2[0].0, BatchId(5));
    }
}
