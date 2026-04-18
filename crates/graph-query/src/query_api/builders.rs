use graph_core::{BatchId, InfluenceKindId, LocusId, LocusKindId};
use graph_world::World;

use super::{
    EntityPredicate, EntitySort, LocusPredicate, LocusSort, Query, QueryResult, RelSort,
    RelationshipPredicate, execute,
};

#[derive(Debug, Clone, Default)]
pub struct FindRelationshipsBuilder {
    predicates: Vec<RelationshipPredicate>,
    sort_by: Option<RelSort>,
    limit: Option<usize>,
}

impl FindRelationshipsBuilder {
    pub fn of_kind(mut self, kind: InfluenceKindId) -> Self {
        self.predicates.push(RelationshipPredicate::OfKind(kind));
        self
    }

    pub fn from_locus(mut self, locus: LocusId) -> Self {
        self.predicates.push(RelationshipPredicate::From(locus));
        self
    }

    pub fn to_locus(mut self, locus: LocusId) -> Self {
        self.predicates.push(RelationshipPredicate::To(locus));
        self
    }

    pub fn touching(mut self, locus: LocusId) -> Self {
        self.predicates.push(RelationshipPredicate::Touching(locus));
        self
    }

    pub fn activity_above(mut self, min: f32) -> Self {
        self.predicates
            .push(RelationshipPredicate::ActivityAbove(min));
        self
    }

    pub fn strength_above(mut self, min: f32) -> Self {
        self.predicates
            .push(RelationshipPredicate::StrengthAbove(min));
        self
    }

    pub fn slot_above(mut self, slot: usize, min: f32) -> Self {
        self.predicates
            .push(RelationshipPredicate::SlotAbove { slot, min });
        self
    }

    pub fn created_in_range(mut self, from: BatchId, to: BatchId) -> Self {
        self.predicates
            .push(RelationshipPredicate::CreatedInRange { from, to });
        self
    }

    pub fn older_than(mut self, current_batch: BatchId, min_batches: u64) -> Self {
        self.predicates.push(RelationshipPredicate::OlderThan {
            current_batch,
            min_batches,
        });
        self
    }

    pub fn min_change_count(mut self, min: u64) -> Self {
        self.predicates
            .push(RelationshipPredicate::MinChangeCount(min));
        self
    }

    pub fn sort_by(mut self, sort: RelSort) -> Self {
        self.sort_by = Some(sort);
        self
    }

    pub fn limit(mut self, n: usize) -> Self {
        self.limit = Some(n);
        self
    }

    pub fn build(self) -> Query {
        Query::FindRelationships {
            predicates: self.predicates,
            sort_by: self.sort_by,
            limit: self.limit,
        }
    }

    pub fn run(self, world: &World) -> QueryResult {
        execute(world, &self.build())
    }
}

#[derive(Debug, Clone, Default)]
pub struct FindLociBuilder {
    predicates: Vec<LocusPredicate>,
    sort_by: Option<LocusSort>,
    limit: Option<usize>,
}

impl FindLociBuilder {
    pub fn of_kind(mut self, kind: LocusKindId) -> Self {
        self.predicates.push(LocusPredicate::OfKind(kind));
        self
    }

    pub fn state_above(mut self, slot: usize, min: f32) -> Self {
        self.predicates
            .push(LocusPredicate::StateAbove { slot, min });
        self
    }

    pub fn state_below(mut self, slot: usize, max: f32) -> Self {
        self.predicates
            .push(LocusPredicate::StateBelow { slot, max });
        self
    }

    pub fn min_degree(mut self, min: usize) -> Self {
        self.predicates.push(LocusPredicate::MinDegree(min));
        self
    }

    pub fn str_property_eq(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.predicates.push(LocusPredicate::StrPropertyEq {
            key: key.into(),
            value: value.into(),
        });
        self
    }

    pub fn f64_property_above(mut self, key: impl Into<String>, min: f64) -> Self {
        self.predicates.push(LocusPredicate::F64PropertyAbove {
            key: key.into(),
            min,
        });
        self
    }

    pub fn reachable_from(mut self, start: LocusId, depth: usize) -> Self {
        self.predicates
            .push(LocusPredicate::ReachableFrom { start, depth });
        self
    }

    pub fn downstream_of(mut self, start: LocusId, depth: usize) -> Self {
        self.predicates
            .push(LocusPredicate::DownstreamOf { start, depth });
        self
    }

    pub fn upstream_of(mut self, start: LocusId, depth: usize) -> Self {
        self.predicates
            .push(LocusPredicate::UpstreamOf { start, depth });
        self
    }

    pub fn reachable_from_active(
        mut self,
        start: LocusId,
        depth: usize,
        min_activity: f32,
    ) -> Self {
        self.predicates.push(LocusPredicate::ReachableFromActive {
            start,
            depth,
            min_activity,
        });
        self
    }

    pub fn downstream_of_active(mut self, start: LocusId, depth: usize, min_activity: f32) -> Self {
        self.predicates.push(LocusPredicate::DownstreamOfActive {
            start,
            depth,
            min_activity,
        });
        self
    }

    pub fn upstream_of_active(mut self, start: LocusId, depth: usize, min_activity: f32) -> Self {
        self.predicates.push(LocusPredicate::UpstreamOfActive {
            start,
            depth,
            min_activity,
        });
        self
    }

    pub fn sort_by(mut self, sort: LocusSort) -> Self {
        self.sort_by = Some(sort);
        self
    }

    pub fn limit(mut self, n: usize) -> Self {
        self.limit = Some(n);
        self
    }

    pub fn build(self) -> Query {
        Query::FindLoci {
            predicates: self.predicates,
            sort_by: self.sort_by,
            limit: self.limit,
        }
    }

    pub fn run(self, world: &World) -> QueryResult {
        execute(world, &self.build())
    }
}

#[derive(Debug, Clone, Default)]
pub struct FindEntitiesBuilder {
    predicates: Vec<EntityPredicate>,
    sort_by: Option<EntitySort>,
    limit: Option<usize>,
}

impl FindEntitiesBuilder {
    pub fn coherence_above(mut self, min: f32) -> Self {
        self.predicates.push(EntityPredicate::CoherenceAbove(min));
        self
    }

    pub fn has_member(mut self, locus: LocusId) -> Self {
        self.predicates.push(EntityPredicate::HasMember(locus));
        self
    }

    pub fn min_members(mut self, min: usize) -> Self {
        self.predicates.push(EntityPredicate::MinMembers(min));
        self
    }

    pub fn sort_by(mut self, sort: EntitySort) -> Self {
        self.sort_by = Some(sort);
        self
    }

    pub fn limit(mut self, n: usize) -> Self {
        self.limit = Some(n);
        self
    }

    pub fn build(self) -> Query {
        Query::FindEntities {
            predicates: self.predicates,
            sort_by: self.sort_by,
            limit: self.limit,
        }
    }

    pub fn run(self, world: &World) -> QueryResult {
        execute(world, &self.build())
    }
}

impl Query {
    pub fn find_relationships() -> FindRelationshipsBuilder {
        FindRelationshipsBuilder::default()
    }

    pub fn find_loci() -> FindLociBuilder {
        FindLociBuilder::default()
    }

    pub fn find_entities() -> FindEntitiesBuilder {
        FindEntitiesBuilder::default()
    }
}
