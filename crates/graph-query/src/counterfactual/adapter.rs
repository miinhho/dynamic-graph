use graph_core::{BatchId, ChangeId, RelationshipId};
use graph_world::World;

use crate::causality::causal_descendants;

pub(super) struct CounterfactualWorldView<'w> {
    world: &'w World,
}

impl<'w> CounterfactualWorldView<'w> {
    pub(super) fn new(world: &'w World) -> Self {
        Self { world }
    }

    pub(super) fn descendants_of(&self, root: ChangeId) -> Vec<ChangeId> {
        causal_descendants(self.world, root)
    }

    pub(super) fn relationship_ids_created_by_descendants(
        &self,
        suppressed: &rustc_hash::FxHashSet<ChangeId>,
    ) -> Vec<RelationshipId> {
        self.world
            .relationships()
            .iter()
            .filter(|relationship| {
                relationship
                    .lineage
                    .created_by
                    .is_some_and(|change_id| suppressed.contains(&change_id))
            })
            .map(|relationship| relationship.id)
            .collect()
    }

    pub(super) fn relationship_ids_touched_by_descendants(
        &self,
        descendants: &rustc_hash::FxHashSet<ChangeId>,
    ) -> Vec<RelationshipId> {
        descendants
            .iter()
            .filter_map(|&change_id| self.world.log().get(change_id))
            .filter_map(|change| match change.subject {
                graph_core::ChangeSubject::Relationship(rel_id) => Some(rel_id),
                graph_core::ChangeSubject::Locus(_) => None,
            })
            .collect()
    }

    pub(super) fn divergence_batch(
        &self,
        suppressed: &rustc_hash::FxHashSet<ChangeId>,
    ) -> Option<BatchId> {
        suppressed
            .iter()
            .filter_map(|&change_id| self.world.log().get(change_id))
            .map(|change| change.batch)
            .min()
    }

    pub(super) fn batch_change_ids(&self, batch: BatchId) -> Vec<ChangeId> {
        self.world
            .log()
            .batch(batch)
            .map(|change| change.id)
            .collect()
    }
}
