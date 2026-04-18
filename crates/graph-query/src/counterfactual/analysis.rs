use graph_core::{BatchId, ChangeId, RelationshipId};
use rustc_hash::FxHashSet;

use super::CounterfactualDiff;
use super::adapter::CounterfactualWorldView;

pub fn relationships_caused_by(
    world: &graph_world::World,
    root_changes: &[ChangeId],
) -> FxHashSet<RelationshipId> {
    let view = CounterfactualWorldView::new(world);
    let all_descendants = collect_descendants(&view, root_changes);
    let mut result: FxHashSet<RelationshipId> = FxHashSet::default();

    for rel_id in view.relationship_ids_touched_by_descendants(&all_descendants) {
        result.insert(rel_id);
    }

    for rel_id in view.relationship_ids_created_by_descendants(&all_descendants) {
        result.insert(rel_id);
    }

    result
}

pub fn relationships_absent_without(
    world: &graph_world::World,
    root_changes: &[ChangeId],
) -> Vec<RelationshipId> {
    let view = CounterfactualWorldView::new(world);
    let all_descendants = collect_descendants(&view, root_changes);
    view.relationship_ids_created_by_descendants(&all_descendants)
}

pub fn counterfactual_replay(
    world: &graph_world::World,
    remove: Vec<ChangeId>,
) -> CounterfactualDiff {
    let view = CounterfactualWorldView::new(world);
    let suppressed = collect_descendants(&view, &remove);
    let absent_relationships = view.relationship_ids_created_by_descendants(&suppressed);
    let divergence_batch = view.divergence_batch(&suppressed);

    CounterfactualDiff {
        removed_roots: remove,
        suppressed_changes: suppressed,
        absent_relationships,
        divergence_batch,
    }
}

pub(crate) fn world_batch_changes(world: &graph_world::World, batch: BatchId) -> Vec<ChangeId> {
    CounterfactualWorldView::new(world).batch_change_ids(batch)
}

fn collect_descendants(
    view: &CounterfactualWorldView<'_>,
    root_changes: &[ChangeId],
) -> FxHashSet<ChangeId> {
    let mut all: FxHashSet<ChangeId> = FxHashSet::default();
    for &root in root_changes {
        all.insert(root);
        for descendant in view.descendants_of(root) {
            all.insert(descendant);
        }
    }
    all
}
