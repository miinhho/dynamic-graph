use graph_core::{Endpoints, EntityId, LocusId};
use graph_world::World;

use crate::query_api::{EntityPredicate, LocusPredicate, RelationshipPredicate};

pub(super) fn graph_locus_members(world: &World, predicate: &LocusPredicate) -> Option<Vec<LocusId>> {
    use crate::traversal::{
        downstream_of, downstream_of_active, reachable_from, reachable_from_active, upstream_of,
        upstream_of_active,
    };

    match predicate {
        LocusPredicate::OfKind(_)
        | LocusPredicate::StateAbove { .. }
        | LocusPredicate::StateBelow { .. }
        | LocusPredicate::StrPropertyEq { .. }
        | LocusPredicate::F64PropertyAbove { .. }
        | LocusPredicate::MinDegree(_) => None,
        LocusPredicate::ReachableFromActive {
            start,
            depth,
            min_activity,
        } => Some(reachable_from_active(world, *start, *depth, *min_activity)),
        LocusPredicate::DownstreamOfActive {
            start,
            depth,
            min_activity,
        } => Some(downstream_of_active(world, *start, *depth, *min_activity)),
        LocusPredicate::UpstreamOfActive {
            start,
            depth,
            min_activity,
        } => Some(upstream_of_active(world, *start, *depth, *min_activity)),
        LocusPredicate::ReachableFrom { start, depth } => Some(reachable_from(world, *start, *depth)),
        LocusPredicate::DownstreamOf { start, depth } => Some(downstream_of(world, *start, *depth)),
        LocusPredicate::UpstreamOf { start, depth } => Some(upstream_of(world, *start, *depth)),
    }
}

pub(super) fn locus_predicate_matches(world: &World, id: LocusId, predicate: &LocusPredicate) -> bool {
    match predicate {
        LocusPredicate::OfKind(kind) => world.locus(id).is_some_and(|l| l.kind == *kind),
        LocusPredicate::StateAbove { slot, min } => {
            locus_slot(world, id, *slot).is_some_and(|value| value >= *min)
        }
        LocusPredicate::StateBelow { slot, max } => {
            locus_slot(world, id, *slot).is_some_and(|value| value <= *max)
        }
        LocusPredicate::StrPropertyEq { key, value } => world
            .properties()
            .get(id)
            .and_then(|props| props.get_str(key))
            .is_some_and(|found| found == value.as_str()),
        LocusPredicate::F64PropertyAbove { key, min } => world
            .properties()
            .get(id)
            .and_then(|props| props.get_f64(key))
            .is_some_and(|found| found >= *min),
        LocusPredicate::MinDegree(min) => world.degree(id) >= *min,
        LocusPredicate::ReachableFromActive { .. }
        | LocusPredicate::DownstreamOfActive { .. }
        | LocusPredicate::UpstreamOfActive { .. }
        | LocusPredicate::ReachableFrom { .. }
        | LocusPredicate::DownstreamOf { .. }
        | LocusPredicate::UpstreamOf { .. } => false,
    }
}

pub(super) fn entity_predicate_matches(world: &World, id: EntityId, predicate: &EntityPredicate) -> bool {
    match predicate {
        EntityPredicate::CoherenceAbove(min) => world
            .entities()
            .get(id)
            .is_some_and(|entity| entity.current.coherence >= *min),
        EntityPredicate::HasMember(locus) => world
            .entities()
            .get(id)
            .is_some_and(|entity| entity.current.members.contains(locus)),
        EntityPredicate::MinMembers(min) => world
            .entities()
            .get(id)
            .is_some_and(|entity| entity.current.members.len() >= *min),
    }
}

pub(super) fn rel_pred_matches(
    relationship: &graph_core::Relationship,
    predicate: &RelationshipPredicate,
) -> bool {
    match predicate {
        RelationshipPredicate::OfKind(kind) => relationship.kind == *kind,
        RelationshipPredicate::From(locus) => {
            matches!(relationship.endpoints, Endpoints::Directed { from, .. } if from == *locus)
        }
        RelationshipPredicate::To(locus) => {
            matches!(relationship.endpoints, Endpoints::Directed { to, .. } if to == *locus)
        }
        RelationshipPredicate::Touching(locus) => relationship.endpoints.involves(*locus),
        RelationshipPredicate::ActivityAbove(min) => relationship.activity() > *min,
        RelationshipPredicate::StrengthAbove(min) => relationship.strength() > *min,
        RelationshipPredicate::SlotAbove { slot, min } => relationship
            .state
            .as_slice()
            .get(*slot)
            .is_some_and(|&value| value >= *min),
        RelationshipPredicate::CreatedInRange { from, to } => {
            relationship.created_batch >= *from && relationship.created_batch <= *to
        }
        RelationshipPredicate::OlderThan {
            current_batch,
            min_batches,
        } => relationship.age_in_batches(*current_batch) >= *min_batches,
        RelationshipPredicate::MinChangeCount(min) => relationship.lineage.change_count >= *min,
    }
}

fn locus_slot(world: &World, id: LocusId, slot: usize) -> Option<f32> {
    world
        .locus(id)
        .and_then(|locus| locus.state.as_slice().get(slot).copied())
}
