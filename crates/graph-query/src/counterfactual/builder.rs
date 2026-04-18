use graph_core::ChangeId;
use graph_world::World;
use rustc_hash::FxHashSet;

use super::analysis;
use super::{relationships_absent_without, relationships_caused_by};

/// Fluent builder for counterfactual queries.
///
/// Created by [`counterfactual`]. Chain methods to narrow the analysis,
/// then call a terminal to retrieve results.
///
/// ## Example
///
/// ```ignore
/// let absent = graph_query::counterfactual(&world)
///     .stimuli_from_batch(batch_id)
///     .relationships_absent_without();
/// ```
pub struct CounterfactualQuery<'w> {
    world: &'w World,
    roots: Vec<ChangeId>,
}

impl<'w> CounterfactualQuery<'w> {
    pub(crate) fn new(world: &'w World) -> Self {
        Self {
            world,
            roots: Vec::new(),
        }
    }

    pub fn with_stimuli(mut self, changes: &[ChangeId]) -> Self {
        self.roots.extend_from_slice(changes);
        self
    }

    pub fn stimuli_from_batch(mut self, batch: graph_core::BatchId) -> Self {
        self.roots
            .extend(analysis::world_batch_changes(self.world, batch));
        self
    }

    pub fn relationships_caused(self) -> FxHashSet<graph_core::RelationshipId> {
        relationships_caused_by(self.world, &self.roots)
    }

    pub fn relationships_absent_without(self) -> Vec<graph_core::RelationshipId> {
        relationships_absent_without(self.world, &self.roots)
    }
}

/// Start a counterfactual query over `world`.
///
/// ```ignore
/// let q = graph_query::counterfactual(&world)
///     .stimuli_from_batch(batch_id)
///     .relationships_absent_without();
/// ```
pub fn counterfactual(world: &World) -> CounterfactualQuery<'_> {
    CounterfactualQuery::new(world)
}
