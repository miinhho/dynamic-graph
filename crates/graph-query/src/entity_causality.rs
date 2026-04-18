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

mod cause;
mod layers;
mod upstream;

pub use cause::{cause_seed_changes, entity_transition_cause};
pub use layers::entity_layers_in_range;
pub use upstream::entity_upstream_transitions;

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{BatchId, EntityId, EntitySnapshot, LayerTransition, LifecycleCause, LocusId};
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
